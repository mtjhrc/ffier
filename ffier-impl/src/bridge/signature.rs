//! C ABI signatures and type resolution for bridge methods.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use crate::meta::{
    MetaMethod, MetaParamKind, MetaReceiver, MetaReturn, MetaTypePair, extract_result_ok_type,
    is_result_ok_handle, option_value_struct_inner,
};

use super::returns::is_borrowed_handle;

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
        // Builder by-value methods take *mut *mut c_void (pointer to the
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
            MetaParamKind::StrSlice
                | MetaParamKind::HandleSlice(_)
                | MetaParamKind::PrimitiveSlice(_)
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

    // Return type + out-param for handle returns or Result
    let ret = match &method.ret {
        MetaReturn::Void => quote! {},
        MetaReturn::Value(vk) => {
            if option_value_struct_inner(&vk.rust_type, value_types).is_some()
                && let Some(inner) = crate::meta::option_inner_tokens(&vk.bridge_type)
            {
                params.push(CExternParam {
                    name: format_ident!("result"),
                    c_type: quote! { *mut <#inner as #lib_crate::FfiType>::CRepr },
                });
                // All values (handles and primitives) returned directly.
                // Handles return *mut c_void, primitives return their CRepr.
                quote! { -> bool }
            } else {
                let ty = c_return_type(vk, lib_crate);
                quote! { -> #ty }
            }
        }
        MetaReturn::HandleSlice { .. } => {
            // &[&T] or &[T] where T is a handle — return FfierObjectArray by value.
            quote! { -> ffier::FfierObjectArray }
        }
        MetaReturn::Result { ok, .. } => {
            // `Result<Option<ValueStruct>, E>` writes presence and value through
            // separate out-params, so neither can be part of the value branch.
            let optional_value = ok.as_ref().and_then(|tp| {
                option_value_struct_inner(&tp.rust_type, value_types)
                    .and_then(|_| crate::meta::option_inner_tokens(&tp.bridge_type))
            });

            // Foreign ok types are always handles (handle-or-null).
            let ok_is_foreign = ok.as_ref().is_some_and(|tp| tp.foreign_crate.is_some());
            let ok_is_handle = ok_is_foreign
                || (ok.is_some() && is_result_ok_handle(&method.rust_ret, handle_types));
            let ok_is_borrowed_handle = !ok_is_foreign && ok.is_some() && {
                let ok_tokens = extract_result_ok_type(&method.rust_ret);
                syn::parse2::<syn::Type>(ok_tokens)
                    .ok()
                    .is_some_and(|ty| is_borrowed_handle(&ty, handle_types))
            };

            // Builder by-value Result<Self, E>: the ok value is written back
            // through the double-pointer handle param, not returned. Use
            // FtResult style, not handle-or-null.
            let is_builder_self_result =
                method.is_builder() && method.receiver == MetaReceiver::Value;

            if let Some(inner) = optional_value {
                params.push(CExternParam {
                    name: format_ident!("result_is_some"),
                    c_type: quote! { *mut bool },
                });
                params.push(CExternParam {
                    name: format_ident!("result"),
                    c_type: quote! { *mut <#inner as #lib_crate::FfiType>::CRepr },
                });
                params.push(CExternParam {
                    name: format_ident!("err_out"),
                    c_type: quote! { *mut *mut core::ffi::c_void },
                });
                quote! { -> ffier::FfierResult }
            } else if (ok_is_handle || ok_is_borrowed_handle) && !is_builder_self_result {
                // Handle-or-null: return handle directly (NULL on error).
                // err_out is *mut *mut c_void (pointer to caller's FtError variable).
                params.push(CExternParam {
                    name: format_ident!("err_out"),
                    c_type: quote! { *mut *mut core::ffi::c_void },
                });
                quote! { -> *mut core::ffi::c_void }
            } else {
                // FtResult style: return packed error code.
                // Builder-self-result writes ok back through the handle
                // double-pointer, so no separate result out-param.
                if !is_builder_self_result && let Some(vk) = ok {
                    params.push(CExternParam {
                        name: format_ident!("result"),
                        c_type: c_out_param_type(vk, lib_crate),
                    });
                }
                // err_out is *mut *mut c_void (pointer to caller's FtError variable).
                params.push(CExternParam {
                    name: format_ident!("err_out"),
                    c_type: quote! { *mut *mut core::ffi::c_void },
                });
                quote! { -> ffier::FfierResult }
            }
        }
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
        MetaParamKind::HandleSlice(_) => quote! { *const *mut core::ffi::c_void },
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
