//! How each native value crosses the C ABI, defined once for both
//! directions.
//!
//! The bridge (C calls an exported function) lifts params and lowers the
//! return; the vtable caller (Rust calls a C callback) lowers args and lifts
//! the return. Both use the same token conversions for values. Parameter
//! conversion also handles slices and borrows scoped to the call.
//!
//! `impl Trait` params are not values here: the bridge dispatches them
//! (`bridge/params.rs`), and the vtable caller does not support them.

use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::c_abi::token_for;
use crate::meta::{MetaParam, MetaParamKind, MetaTypePair};
use crate::return_shape::is_borrowed_handle;

/// Lift a value through the conversion impl selected by `tok`.
pub(crate) fn lift_value(tok: &TokenStream, value: &TokenStream) -> TokenStream {
    quote! { unsafe { ffier::FromC::<#tok>::from_c(#value) } }
}

/// Lower a value through the conversion impl selected by `tok`.
pub(crate) fn lower_value(tok: &TokenStream, value: &TokenStream) -> TokenStream {
    quote! { ffier::IntoC::<#tok>::into_c(#value) }
}

/// Bindings to emit before the call, and the converted expressions.
#[derive(Default)]
pub(crate) struct Converted {
    pub(crate) pre: Vec<TokenStream>,
    /// One expression per C value (a slice lowers to pointer and length).
    pub(crate) exprs: Vec<TokenStream>,
}

/// A single parameter converted by type: everything except slices and
/// callbacks.
///
/// The Rust type is never spelled: it is inferred where the value is used.
struct Leaf {
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
    /// Classify a type pair. `local_token` names this library's token where
    /// the conversion is emitted.
    fn of(tp: &MetaTypePair, handle_types: &HashSet<String>, local_token: &TokenStream) -> Self {
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
    fn lift(&self, raw: &TokenStream, name: &syn::Ident) -> Converted {
        let tok = &self.tok;
        match &self.kind {
            LeafKind::Token | LeafKind::NativeRef => Converted {
                pre: Vec::new(),
                exprs: vec![lift_value(tok, raw)],
            },
            // A foreign handle is lent for the call: wrap it in its client
            // wrapper without taking ownership.
            LeafKind::ForeignRef { target, mutable } => {
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
        }
    }

    /// Convert a Rust value into its C value to hand to C.
    ///
    /// The value's type is inferred rather than spelled: bridge types erase
    /// lifetimes to `'static`, which a borrowed argument does not satisfy.
    fn lower(&self, value: &TokenStream) -> TokenStream {
        let tok = &self.tok;
        match &self.kind {
            LeafKind::Token => lower_value(tok, value),
            LeafKind::NativeRef => quote! {
                compile_error!("ffier: vtable callbacks cannot take a borrowed handle argument yet")
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
    let lift_slice = || {
        let len = len.expect("slice param has a length param");
        quote! {
            if #len == 0 {
                &[]
            } else {
                unsafe { core::slice::from_raw_parts(#id, #len) }
            }
        }
    };
    match &p.kind {
        MetaParamKind::Regular(tp) => {
            let mut converted = Leaf::of(tp, handle_types, local_token).lift(&quote! { #id }, id);
            converted.pre.splice(0..0, foreign_c_name_assert(tp));
            converted
        }
        MetaParamKind::StrSlice => {
            let slice = lift_slice();
            let binding = format_ident!("__{id}_vec");
            Converted {
                pre: vec![quote! {
                    let #binding: Vec<&str> = {
                        let __slice = #slice;
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
            let slice = lift_slice();
            let binding = format_ident!("__{id}_slice");
            Converted {
                pre: vec![quote! { let #binding = #slice; }],
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

/// Strip the Rust reference from the bridge type of a foreign handle.
fn borrowed_handle_target(tp: &MetaTypePair) -> TokenStream {
    match syn::parse2::<syn::Type>(tp.bridge_type.clone()) {
        Ok(syn::Type::Reference(reference)) => {
            let target = &reference.elem;
            quote! { #target }
        }
        _ => tp.bridge_type.clone(),
    }
}
