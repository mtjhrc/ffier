//! Return-value marshaling for bridge calls.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::conv::{Leaf, Scope};
use crate::meta::MetaReturn;
use crate::return_shape::{ResultConv, ReturnShape, classify_return};

use super::dispatch::ErrorMap;

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
    let local_token = crate::ffi_traits::token_of(lib_crate);
    // Convert a returned value; it outlives the call.
    let lower_ok = |tp: &crate::meta::MetaTypePair, value: TokenStream2| {
        Leaf::of(tp, handle_types, &local_token).lower(&value, Scope::Returned)
    };
    let (ok, err_ident, conv) = match classify_return(ret, rust_ret, handle_types, value_types) {
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
        ReturnShape::Value { tp, .. } => {
            let value = lower_ok(tp, quote! { result });
            return quote! {
                let result = #call_expr;
                #value
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
        // Owned or borrowed (`&Handle`, as a borrowed shell) ok handle.
        ResultConv::OwnedHandleOrNull | ResultConv::BorrowedHandleOrNull => {
            let value = lower_ok(ok.expect("handle-or-null ok type"), quote! { ok_val });
            quote! {
                match #call_expr {
                    Ok(ok_val) => #value,
                    Err(e) => { #box_expr core::ptr::null_mut() }
                }
            }
        }
        ResultConv::OutParam => {
            let ok_branch = match (ok, replaced_self) {
                (Some(tp), _) => {
                    let value = lower_ok(tp, quote! { ok_val });
                    quote! {
                        Ok(ok_val) => {
                            unsafe { result.write(#value) };
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
