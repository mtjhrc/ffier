//! C ABI signatures and type resolution for bridge methods.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use crate::meta::{MetaMethod, MetaParamKind, MetaReceiver, MetaTypePair};
use crate::return_shape::{ResultConv, ReturnShape, classify_return};

/// A single parameter in a C extern signature.
pub(super) struct CExternParam {
    pub(super) name: syn::Ident,
    pub(super) c_type: TokenStream2,
}

/// Complete C extern function signature for a method.
///
/// Contains all the information needed to emit an `unsafe extern "C" { fn ... }`
/// declaration. The parameters include handle, regular params, and out-param
/// (for Result returns) in the order they appear.
pub(super) struct CExternSignature {
    /// Fully qualified extern function name (e.g. "mylib_calculator_add").
    _fn_name: String,
    /// All parameters in declaration order.
    pub(super) params: Vec<CExternParam>,
    /// Return type tokens (empty for void).
    pub(super) ret: TokenStream2,
}

/// Compute the full C extern signature for a method.
///
/// This is the single source of truth for "what does this method look like
/// as an `extern "C"` function".
pub(super) fn c_signature_for_method(
    method: &MetaMethod,
    prefix: &str,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    lib_crate: &TokenStream2,
) -> CExternSignature {
    let fn_name = format!("{}_{}", prefix, method.ffi_name());
    let mut params = Vec::new();

    // Handle param (receiver)
    let has_receiver = method.receiver != MetaReceiver::None;

    if has_receiver {
        // All handles are heap-allocated, passed as *mut c_void.
        // By-value methods of a type whose returned Self replaces the receiver take *mut *mut c_void (pointer to the
        // caller's handle variable) so the bridge can swap the pointer.
        params.push(CExternParam {
            name: format_ident!("handle"),
            c_type: quote! { *mut core::ffi::c_void },
        });
    }

    // Regular params
    for p in &method.params {
        if matches!(
            p.kind,
            MetaParamKind::StrSlice | MetaParamKind::PrimitiveSlice(_)
        ) {
            params.push(CExternParam {
                name: p.name.clone(),
                c_type: c_param_type(&p.kind, lib_crate),
            });
            params.push(CExternParam {
                name: format_ident!("{}_len", p.name),
                c_type: quote! { usize },
            });
        } else {
            params.push(CExternParam {
                name: p.name.clone(),
                c_type: c_param_type(&p.kind, lib_crate),
            });
        }
    }

    // Return type + out-params, from the shared return classification.
    let mut out_param = |name: &str, c_type: TokenStream2| {
        params.push(CExternParam {
            name: format_ident!("{name}"),
            c_type,
        })
    };
    let err_out = quote! { *mut *mut core::ffi::c_void };
    let shape = classify_return(&method.ret, &method.rust_ret, handle_types, value_types);
    let ret = match shape {
        ReturnShape::Void => quote! {},
        ReturnShape::OptionalValue { inner, .. } => {
            out_param(
                "result",
                quote! { *mut <#inner as #lib_crate::FfiType>::CRepr },
            );
            quote! { -> bool }
        }
        // Handles return *mut c_void, everything else its CRepr.
        ReturnShape::Value { tp, .. } => {
            let ty = c_return_type(tp, lib_crate);
            quote! { -> #ty }
        }
        ReturnShape::Result { ok, conv, .. } => match conv {
            ResultConv::OptionalValueOutParam { inner } => {
                out_param("result_is_some", quote! { *mut bool });
                out_param(
                    "result",
                    quote! { *mut <#inner as #lib_crate::FfiType>::CRepr },
                );
                out_param("err_out", err_out);
                quote! { -> ffier::FfierResult }
            }
            ResultConv::OwnedHandleOrNull | ResultConv::BorrowedHandleOrNull => {
                // Return the handle directly, NULL on error.
                out_param("err_out", err_out);
                quote! { -> *mut core::ffi::c_void }
            }
            ResultConv::OutParam => {
                // A returned Self that replaces the receiver has no ok here:
                // it is written back through the handle param.
                if let Some(tp) = ok {
                    out_param("result", c_out_param_type(tp, lib_crate));
                }
                out_param("err_out", err_out);
                quote! { -> ffier::FfierResult }
            }
        },
    };

    CExternSignature {
        _fn_name: fn_name,
        params,
        ret,
    }
}

/// Resolve the crate whose `FfiType`/`FfiHandle` traits to use for a type pair.
/// Returns `foreign_crate` if set, otherwise falls back to `lib_crate`.
pub(super) fn resolve_trait_crate<'a>(
    tp: &'a MetaTypePair,
    lib_crate: &'a TokenStream2,
) -> &'a TokenStream2 {
    tp.foreign_crate.as_ref().unwrap_or(lib_crate)
}

/// Produce the C type tokens for a parameter kind.
fn c_param_type(kind: &MetaParamKind, lib_crate: &TokenStream2) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular(tp) => {
            let bridge_type = &tp.bridge_type;
            let tc = resolve_trait_crate(tp, lib_crate);
            quote! { <#bridge_type as #tc::FfiType>::CRepr }
        }
        MetaParamKind::ImplTrait { .. } => quote! { *mut core::ffi::c_void },
        MetaParamKind::StrSlice => quote! { *const ffier::FfierBytes },
        MetaParamKind::PrimitiveSlice(tp) => {
            let bridge_type = &tp.bridge_type;
            let tc = resolve_trait_crate(tp, lib_crate);
            quote! { *const <#bridge_type as #tc::FfiType>::CRepr }
        }
    }
}

/// Produce the C return type tokens for a value kind.
fn c_return_type(kind: &MetaTypePair, lib_crate: &TokenStream2) -> TokenStream2 {
    let bridge_type = &kind.bridge_type;
    let tc = resolve_trait_crate(kind, lib_crate);
    quote! { <#bridge_type as #tc::FfiType>::CRepr }
}

/// Produce the C type for a Result ok-value out-parameter.
fn c_out_param_type(kind: &MetaTypePair, lib_crate: &TokenStream2) -> TokenStream2 {
    let inner = c_return_type(kind, lib_crate);
    quote! { *mut #inner }
}
