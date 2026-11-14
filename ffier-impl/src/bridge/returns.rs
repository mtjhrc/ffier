//! Return-value marshaling for bridge calls.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::meta::MetaReturn;
use crate::return_shape::{ResultConv, ReturnShape, borrowed_handle_target, classify_return};

use super::dispatch::ErrorMap;
use super::signature::resolve_trait_crate;

/// Wrap a call expression in the appropriate return-value conversion.
///
/// Given an expression that evaluates to the Rust return type, produce tokens
/// that convert it to the C return, following [`classify_return`]. Used by
/// exported methods, free functions, and trait dispatch.
///
/// `replaced_self` is the struct path when the returned `Self` replaces a
/// by-value receiver: the new handle is written back through `__handle_slot`.
#[allow(clippy::too_many_arguments)]
pub(super) fn wrap_return(
    call_expr: TokenStream2,
    ret: &MetaReturn,
    rust_ret: &TokenStream2,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    error_map: &ErrorMap,
    replaced_self: Option<&TokenStream2>,
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    let mut call_expr = call_expr;
    let (ok, err_ident, conv) = match classify_return(ret, rust_ret, handle_types, value_types) {
        // The native function must not return; abort before control comes
        // back to C if it does.
        ReturnShape::Never => {
            return quote! {
                #[allow(unreachable_code)]
                {
                    #call_expr;
                    std::process::abort()
                }
            };
        }
        // The native alias may be inhabited (e.g. `type Never = ()`): abort on
        // success before anything crosses the ABI, then marshal the error like
        // `Result<(), E>`. Matching on `.err()` avoids an `Ok` arm that is
        // unreachable (and linted) when the alias is uninhabited.
        ReturnShape::ResultNeverOk { err_ident } => {
            call_expr = quote! {
                match (#call_expr).err() {
                    Some(__error) => Result::<(), _>::Err(__error),
                    None => std::process::abort(),
                }
            };
            (None, err_ident, ResultConv::OutParam)
        }
        ReturnShape::Void => {
            return match replaced_self {
                // The method returns the new Self: write its handle back.
                Some(struct_path) => quote! {
                    let result = #call_expr;
                    let __new_ptr = <#struct_path as ffier::IntoC<#lib_crate::__FfierLocalToken>>::into_c(result);
                    unsafe { *__handle_slot = __new_ptr };
                },
                None => quote! { #call_expr; },
            };
        }
        // Presence is the C return value; the value goes through an
        // out-param and is written only when present.
        ReturnShape::OptionalValue { inner, .. } => {
            return quote! {
                match #call_expr {
                    Some(__value) => {
                        unsafe {
                            result.write(<#inner as ffier::IntoC<#lib_crate::__FfierLocalToken>>::into_c(__value));
                        }
                        true
                    }
                    None => false,
                }
            };
        }
        ReturnShape::Value {
            tp,
            borrowed_handle,
        } => {
            let tc = resolve_trait_crate(tp, lib_crate);
            if borrowed_handle {
                // &T where T is a handle — shallow-copy into a borrowed handle.
                let target = borrowed_handle_target(tp);
                return quote! {
                    let result = #call_expr;
                    unsafe {
                        ffier::ffier_handle_new_borrowed::<#target>(
                            <#target as ffier::FfiHandle<#tc::__FfierLocalToken>>::TYPE_TAG,
                            result as *const #target,
                        )
                    }
                };
            }
            let bridge_type = &tp.bridge_type;
            return quote! {
                let result = #call_expr;
                <#bridge_type as ffier::IntoC<#tc::__FfierLocalToken>>::into_c(result)
            };
        }
        ReturnShape::Result {
            ok,
            err_ident,
            conv,
        } => (ok, err_ident, conv),
    };

    let Some(err_info) = error_map.get(err_ident) else {
        let msg = format!(
            "method returns Result<_, {err_ident}> but `{err_ident}` is not registered \
             as an error type. Add `#[derive(ffier::FfiError)]` to `{err_ident}`, or wrap \
             the error in a type that derives it."
        );
        return quote! { compile_error!(#msg); };
    };
    let err_type_tag = err_info.type_tag;
    let box_expr = quote! {
        if !err_out.is_null() {
            unsafe { *err_out = <_ as ffier::IntoC<#lib_crate::__FfierLocalToken>>::into_c(e); }
        }
    };
    let ok_tc = ok
        .map(|tp| resolve_trait_crate(tp, lib_crate))
        .unwrap_or(lib_crate);

    match conv {
        // Presence and value go through their own out-params. `None` writes
        // only the flag; errors write only `err_out`, leaving both success
        // outputs untouched.
        ResultConv::OptionalValueOutParam { inner } => quote! {
            match #call_expr {
                Ok(Some(__value)) => {
                    unsafe {
                        result_is_some.write(true);
                        result.write(<#inner as ffier::IntoC<#lib_crate::__FfierLocalToken>>::into_c(__value));
                    }
                    ffier::FFIER_RESULT_SUCCESS
                }
                Ok(None) => {
                    unsafe { result_is_some.write(false) };
                    ffier::FFIER_RESULT_SUCCESS
                }
                Err(e) => {
                    let __r = ffier::ffier_result(
                        #err_type_tag,
                        #lib_crate::FfiError::code(&e),
                    );
                    #box_expr
                    __r
                }
            }
        },
        // Return the owned handle directly, NULL on error.
        ResultConv::OwnedHandleOrNull => quote! {
            match #call_expr {
                Ok(ok_val) => <_ as ffier::IntoC<#ok_tc::__FfierLocalToken>>::into_c(ok_val),
                Err(e) => { #box_expr core::ptr::null_mut() }
            }
        },
        // Result<&Handle, E> → borrowed shell, NULL on error.
        ResultConv::BorrowedHandleOrNull => {
            let target = borrowed_handle_target(ok.expect("borrowed handle ok type"));
            quote! {
                match #call_expr {
                    Ok(ok_val) => unsafe {
                        ffier::ffier_handle_new_borrowed::<#target>(
                            <#target as ffier::FfiHandle<#ok_tc::__FfierLocalToken>>::TYPE_TAG,
                            ok_val as *const #target,
                        )
                    },
                    Err(e) => { #box_expr core::ptr::null_mut() }
                }
            }
        }
        ResultConv::OutParam => {
            let ok_branch = match (ok, replaced_self) {
                (Some(tp), _) => {
                    let bridge_type = &tp.bridge_type;
                    quote! {
                        Ok(ok_val) => {
                            unsafe { result.write(<#bridge_type as ffier::IntoC<#ok_tc::__FfierLocalToken>>::into_c(ok_val)) };
                            ffier::FFIER_RESULT_SUCCESS
                        }
                    }
                }
                (None, Some(struct_path)) => quote! {
                    Ok(new_self) => {
                        let __new_ptr = <#struct_path as ffier::IntoC<#lib_crate::__FfierLocalToken>>::into_c(new_self);
                        unsafe { *__handle_slot = __new_ptr };
                        ffier::FFIER_RESULT_SUCCESS
                    }
                },
                (None, None) => quote! {
                    Ok(_) => ffier::FFIER_RESULT_SUCCESS,
                },
            };
            // The consumed receiver is gone on error: clear the caller's handle.
            let clear_slot = replaced_self.map(|_| {
                quote! { unsafe { *__handle_slot = core::ptr::null_mut() }; }
            });
            quote! {
                match #call_expr {
                    #ok_branch
                    Err(e) => {
                        let __r = ffier::ffier_result(#err_type_tag, #lib_crate::FfiError::code(&e));
                        #clear_slot
                        #box_expr
                        __r
                    }
                }
            }
        }
    }
}
