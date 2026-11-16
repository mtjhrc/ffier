//! How each native value crosses the C ABI, defined once for both
//! directions.
//!
//! The bridge (C calls an exported function) lifts params and lowers the
//! return; the vtable caller (Rust calls a C callback) lowers args and lifts
//! the return. Both go through [`Leaf`] and the param helpers here, so every
//! type kind is converted the same way in either role. [`Scope`] says
//! whether the value is needed only for the call or outlives it, which
//! decides how a borrowed handle is lent.
//!
//! `impl Trait` params are not values here: the bridge dispatches them
//! (`bridge/params.rs`), and the vtable caller does not support them.

use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::c_abi::token_for;
use crate::meta::{MetaParam, MetaParamKind, MetaTypePair};
use crate::return_shape::{borrowed_handle_target, is_borrowed_handle};

/// How long a converted value must live.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Only for the duration of the call: arguments and parameters.
    Call,
    /// Beyond the call: return values.
    Returned,
}

/// Bindings to emit before the call, and the converted expressions.
#[derive(Default)]
pub(crate) struct Converted {
    pub(crate) pre: Vec<TokenStream>,
    /// One expression per C value (a slice lowers to pointer and length).
    pub(crate) exprs: Vec<TokenStream>,
}

/// A single value converted by type: everything except slices and
/// callbacks.
///
/// The Rust type is never spelled: it is inferred where the value is used.
pub(crate) struct Leaf {
    /// The token whose impls convert the value.
    tok: TokenStream,
    kind: LeafKind,
}

enum LeafKind {
    /// Converted by its `IntoC`/`FromC` impls.
    Token,
    /// `&Handle` / `&mut Handle` of one of this library's handles.
    NativeRef,
    /// A reference to another library's client wrapper.
    ForeignRef { target: TokenStream, mutable: bool },
}

impl Leaf {
    /// A value converted by its `IntoC`/`FromC` impls under `tok`.
    pub(crate) fn token(tok: TokenStream) -> Self {
        Self {
            tok,
            kind: LeafKind::Token,
        }
    }

    /// Classify a type pair. `local_token` names this library's token where
    /// the conversion is emitted.
    pub(crate) fn of(
        tp: &MetaTypePair,
        handle_types: &HashSet<String>,
        local_token: &TokenStream,
    ) -> Self {
        let tok = token_for(tp, local_token);
        let parsed = syn::parse2::<syn::Type>(tp.rust_type.clone()).ok();
        let reference = match &parsed {
            Some(syn::Type::Reference(r)) => Some(r.mutability.is_some()),
            _ => None,
        };
        let kind = match reference {
            Some(mutable) if tp.foreign_crate.is_some() => LeafKind::ForeignRef {
                target: borrowed_handle_target(tp),
                mutable,
            },
            Some(_)
                if parsed
                    .as_ref()
                    .is_some_and(|ty| is_borrowed_handle(ty, handle_types)) =>
            {
                LeafKind::NativeRef
            }
            _ => LeafKind::Token,
        };
        Self { tok, kind }
    }

    /// Convert a C value received from C into Rust. `name` seeds the names
    /// of any bindings.
    ///
    /// Like [`lower`](Self::lower), the Rust type is inferred from where the
    /// value is used: bridge types erase lifetimes to `'static`, which an
    /// invariant handle cannot shorten to the caller's lifetime.
    pub(crate) fn lift(&self, raw: &TokenStream, scope: Scope, name: &syn::Ident) -> Converted {
        let tok = &self.tok;
        match &self.kind {
            LeafKind::Token | LeafKind::NativeRef => Converted {
                pre: Vec::new(),
                exprs: vec![quote! { unsafe { ffier::FromC::<#tok>::from_c(#raw) } }],
            },
            // A foreign handle is lent for the call: wrap it in its client
            // wrapper without taking ownership.
            LeafKind::ForeignRef { target, mutable } if scope == Scope::Call => {
                let binding = format_ident!("__{name}_foreign");
                let (mutability, borrow) = if *mutable {
                    (quote! { mut }, quote! { &mut * })
                } else {
                    (quote! {}, quote! { &* })
                };
                Converted {
                    pre: vec![quote! {
                        let #mutability #binding = core::mem::ManuallyDrop::new(
                            <#target as ffier::ClientHandle<#tok>>::from_raw(#raw));
                    }],
                    exprs: vec![quote! { #borrow #binding }],
                }
            }
            LeafKind::ForeignRef { .. } => Converted {
                pre: Vec::new(),
                exprs: vec![quote! {
                    compile_error!("ffier: a borrowed foreign handle cannot be received as a return value")
                }],
            },
        }
    }

    /// Convert a Rust value into its C value to hand to C.
    ///
    /// The value's type is inferred rather than spelled: bridge types erase
    /// lifetimes to `'static`, which a borrowed argument does not satisfy.
    pub(crate) fn lower(&self, value: &TokenStream) -> TokenStream {
        let tok = &self.tok;
        match &self.kind {
            LeafKind::Token => quote! { ffier::IntoC::<#tok>::into_c(#value) },
            // Returns are rejected separately (`reject_borrowed_handle_return`);
            // vtable arguments are not supported yet.
            LeafKind::NativeRef => quote! {
                compile_error!("ffier: a borrowed handle can be neither returned nor passed to a vtable callback")
            },
            // The client wrapper holds the foreign handle pointer.
            LeafKind::ForeignRef { target, .. } => quote! {
                <#target as ffier::ClientHandle<#tok>>::as_handle(#value)
            },
        }
    }
}

/// Lift a parameter received from C. `len` names the length param of a
/// slice. `impl Trait` params pass through for the caller to dispatch.
pub(crate) fn lift_param(
    p: &MetaParam,
    len: Option<&syn::Ident>,
    handle_types: &HashSet<String>,
    local_token: &TokenStream,
) -> Converted {
    let id = &p.name;
    match &p.kind {
        MetaParamKind::Regular(tp) => {
            let mut converted =
                Leaf::of(tp, handle_types, local_token).lift(&quote! { #id }, Scope::Call, id);
            converted.pre.splice(0..0, foreign_c_name_assert(tp));
            converted
        }
        MetaParamKind::StrSlice => {
            let len = len.expect("slice param has a length param");
            let binding = format_ident!("__{id}_vec");
            Converted {
                pre: vec![quote! {
                    let #binding: Vec<&str> = {
                        let __slice = unsafe { core::slice::from_raw_parts(#id, #len) };
                        __slice.iter().map(|b| unsafe {
                            core::str::from_utf8_unchecked(
                                core::slice::from_raw_parts(b.data, b.len))
                        }).collect()
                    };
                }],
                exprs: vec![quote! { &#binding }],
            }
        }
        MetaParamKind::PrimitiveSlice(_) => {
            let len = len.expect("slice param has a length param");
            let binding = format_ident!("__{id}_slice");
            Converted {
                pre: vec![quote! {
                    let #binding = if #len == 0 {
                        &[]
                    } else {
                        unsafe { core::slice::from_raw_parts(#id, #len) }
                    };
                }],
                exprs: vec![quote! { #binding }],
            }
        }
        MetaParamKind::ImplTrait { .. } => Converted {
            pre: Vec::new(),
            exprs: vec![quote! { #id }],
        },
    }
}

/// Lower an argument passed to C for the duration of the call.
pub(crate) fn lower_arg(
    p: &MetaParam,
    handle_types: &HashSet<String>,
    local_token: &TokenStream,
) -> Converted {
    let id = &p.name;
    match &p.kind {
        MetaParamKind::Regular(tp) => Converted {
            pre: Vec::new(),
            exprs: vec![Leaf::of(tp, handle_types, local_token).lower(&quote! { #id })],
        },
        MetaParamKind::StrSlice => {
            let binding = format_ident!("__{id}_ffierbytes");
            Converted {
                pre: vec![quote! {
                    let #binding: Vec<ffier::FfierBytes> = #id.iter()
                        .map(|s| unsafe { ffier::FfierBytes::from_str(s) })
                        .collect();
                }],
                exprs: vec![quote! { #binding.as_ptr() }, quote! { #binding.len() }],
            }
        }
        // Primitives are layout-compatible with their CRepr.
        MetaParamKind::PrimitiveSlice(_) => Converted {
            pre: Vec::new(),
            exprs: vec![quote! { #id.as_ptr() }, quote! { #id.len() }],
        },
        MetaParamKind::ImplTrait { .. } => Converted {
            pre: Vec::new(),
            exprs: vec![quote! { core::ptr::null_mut() }],
        },
    }
}

/// For a foreign handle, a compile-time check that the user-provided C
/// name matches the foreign crate's.
fn foreign_c_name_assert(tp: &MetaTypePair) -> Option<TokenStream> {
    let foreign_crate = tp.foreign_crate.as_ref()?;
    let c_name = tp.foreign_c_name.as_ref()?;
    let target = borrowed_handle_target(tp);
    let tok = crate::ffi_traits::token_of(foreign_crate);
    let msg = format!("foreign c_name mismatch for `{target}`: expected \"{c_name}\"");
    Some(quote! {
        const _: () = assert!(
            ffier::const_str_eq(<#target as ffier::FfiHandle<#tok>>::C_HANDLE_NAME, #c_name),
            #msg,
        );
    })
}
