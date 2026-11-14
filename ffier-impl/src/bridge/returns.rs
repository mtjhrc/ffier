//! Return-value marshaling for bridge calls.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::meta::{
    MetaReturn, extract_result_ok_type, is_result_ok_handle, option_value_struct_inner,
};

use super::dispatch::ErrorMap;
use super::signature::resolve_trait_crate;

/// Check if a type (parsed from rust_ret tokens) is `&T` or `&mut T` where
/// `T` is a handle type. Used to emit `ffier_handle_new_borrowed` for
/// borrowed handle returns.
pub(super) fn is_borrowed_handle(ty: &syn::Type, handle_types: &HashSet<String>) -> bool {
    if let syn::Type::Reference(ref_ty) = ty {
        crate::meta::type_last_name(&ref_ty.elem)
            .map(|name| name == "Self" || handle_types.contains(&name))
            .unwrap_or(false)
    } else {
        false
    }
}

/// Optional builder context for methods that consume `self` and write
/// the new handle back through a double pointer.
pub(super) struct BuilderCtx<'a> {
    pub(super) struct_path: &'a TokenStream2,
    pub(super) is_by_value: bool,
}

/// Wrap a call expression in the appropriate return-value conversion.
///
/// This is the single source of truth for "given an expression that evaluates
/// to the Rust return type, produce tokens that convert it to the C return".
/// Used by exported methods, free functions, and trait dispatch.
#[allow(clippy::too_many_arguments)]
pub(super) fn wrap_return(
    call_expr: TokenStream2,
    ret: &MetaReturn,
    rust_ret: &TokenStream2,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    error_map: &ErrorMap,
    builder: Option<&BuilderCtx>,
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    match ret {
        MetaReturn::Void => {
            if let Some(b) = builder.filter(|b| b.is_by_value) {
                // Builder by-value void: method returns Self, write back new handle.
                let struct_path = b.struct_path;
                quote! {
                    let result = #call_expr;
                    let __new_ptr = <#struct_path as #lib_crate::FfiType>::into_c(result);
                    unsafe { *__handle_slot = __new_ptr };
                }
            } else {
                quote! { #call_expr; }
            }
        }
        MetaReturn::Value(tp) => {
            // `Option<ValueStruct>`: presence is the C return value, the value
            // goes through an out-param and is written only when present.
            if option_value_struct_inner(&tp.rust_type, value_types).is_some()
                && let Some(inner) = crate::meta::option_inner_tokens(&tp.bridge_type)
            {
                return quote! {
                    match #call_expr {
                        Some(__value) => {
                            unsafe {
                                result.write(<#inner as #lib_crate::FfiType>::into_c(__value));
                            }
                            true
                        }
                        None => false,
                    }
                };
            }
            let bridge_type = &tp.bridge_type;
            let tc = resolve_trait_crate(tp, lib_crate);
            let rust_ty = syn::parse2::<syn::Type>(rust_ret.clone()).ok();
            // Foreign types are always handles.
            let is_foreign = tp.foreign_crate.is_some();
            if !is_foreign
                && rust_ty
                    .as_ref()
                    .is_some_and(|ty| is_borrowed_handle(ty, handle_types))
            {
                // &T where T is a handle — shallow-copy into a borrowed handle.
                // Strip the reference to get the inner bridge_type for the
                // FfiHandle::TYPE_TAG lookup.
                let inner_bridge = if let Ok(syn::Type::Reference(ref_ty)) =
                    syn::parse2::<syn::Type>(bridge_type.clone())
                {
                    let elem = &ref_ty.elem;
                    quote! { #elem }
                } else {
                    bridge_type.clone()
                };
                quote! {
                    let result = #call_expr;
                    unsafe {
                        ffier::ffier_handle_new_borrowed::<#inner_bridge>(
                            <#inner_bridge as #tc::FfiHandle>::TYPE_TAG,
                            result as *const #inner_bridge,
                        )
                    }
                }
            } else {
                quote! {
                    let result = #call_expr;
                    <#bridge_type as #tc::FfiType>::into_c(result)
                }
            }
        }
        MetaReturn::HandleSlice { types: tp, direct } => {
            let bridge_type = &tp.bridge_type;
            let tc = resolve_trait_crate(tp, lib_crate);
            // &[&T] or &[T] → build a contiguous Box<[FfierBorrowedHandle]>
            // from the iterator, leak it, return as FfierObjectArray.
            // For &[&T], item is &&T so we deref once; for &[T], item is &T directly.
            let ptr_expr = if *direct {
                quote! { item as *const #bridge_type as *const core::ffi::c_void }
            } else {
                quote! { *item as *const #bridge_type as *const core::ffi::c_void }
            };
            quote! {
                let __slice = #call_expr;
                if __slice.is_empty() {
                    ffier::FfierObjectArray::EMPTY
                } else {
                    let __tag = <#bridge_type as #tc::FfiHandle>::TYPE_TAG;
                    let __meta = ffier::METADATA_INDIRECT;
                    let __boxed: Box<[ffier::FfierBorrowedHandle]> = __slice.iter()
                        .map(|item| ffier::FfierBorrowedHandle {
                            type_tag: __tag,
                            metadata: __meta,
                            ptr: #ptr_expr,
                        })
                        .collect();
                    let __len = __boxed.len();
                    let __raw = Box::into_raw(__boxed) as *const ffier::FfierBorrowedHandle;
                    ffier::FfierObjectArray::from_raw(__raw, __len)
                }
            }
        }
        MetaReturn::Result { ok, err_ident } => {
            // A foreign ok type is always a handle.
            let ok_is_foreign = ok.as_ref().is_some_and(|tp| tp.foreign_crate.is_some());
            let ok_is_handle =
                ok_is_foreign || (ok.is_some() && is_result_ok_handle(rust_ret, handle_types));

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
                    unsafe { *err_out = <_ as #lib_crate::FfiType>::into_c(e); }
                }
            };

            // `Result<Option<ValueStruct>, E>`: presence and value go through
            // their own out-params. `None` writes only the flag; errors write
            // only `err_out`, leaving both success outputs untouched.
            if let Some(tp) = ok
                && option_value_struct_inner(&tp.rust_type, value_types).is_some()
                && let Some(inner) = crate::meta::option_inner_tokens(&tp.bridge_type)
            {
                return quote! {
                    match #call_expr {
                        Ok(Some(__value)) => {
                            unsafe {
                                result_is_some.write(true);
                                result.write(<#inner as #lib_crate::FfiType>::into_c(__value));
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
                };
            }

            // Check if the Ok type is a borrowed handle (&HandleType).
            let ok_is_borrowed_handle = !ok_is_foreign && ok.is_some() && {
                let ok_tokens = extract_result_ok_type(rust_ret);
                syn::parse2::<syn::Type>(ok_tokens)
                    .ok()
                    .is_some_and(|ty| is_borrowed_handle(&ty, handle_types))
            };

            // Resolve the trait crate for the ok type (foreign or local).
            let ok_tc = ok
                .as_ref()
                .map(|tp| resolve_trait_crate(tp, lib_crate))
                .unwrap_or(lib_crate);

            if ok_is_handle && !ok_is_borrowed_handle {
                // Return owned handle directly, NULL on error.
                quote! {
                    match #call_expr {
                        Ok(ok_val) => <_ as #ok_tc::FfiType>::into_c(ok_val),
                        Err(e) => { #box_expr core::ptr::null_mut() }
                    }
                }
            } else if ok_is_borrowed_handle {
                // Borrowed handle return: Result<&Handle, E> → new_borrowed, NULL on error.
                let ok_pair = ok.as_ref().unwrap();
                let inner_bridge = if let Ok(syn::Type::Reference(ref_ty)) =
                    syn::parse2::<syn::Type>(ok_pair.bridge_type.clone())
                {
                    let elem = &ref_ty.elem;
                    quote! { #elem }
                } else {
                    ok_pair.bridge_type.clone()
                };
                quote! {
                    match #call_expr {
                        Ok(ok_val) => unsafe {
                            ffier::ffier_handle_new_borrowed::<#inner_bridge>(
                                <#inner_bridge as #ok_tc::FfiHandle>::TYPE_TAG,
                                ok_val as *const #inner_bridge,
                            )
                        },
                        Err(e) => { #box_expr core::ptr::null_mut() }
                    }
                }
            } else {
                let ok_branch = match ok {
                    Some(tp) => {
                        let bridge_type = &tp.bridge_type;
                        quote! {
                            Ok(ok_val) => {
                                unsafe { result.write(<#bridge_type as #ok_tc::FfiType>::into_c(ok_val)) };
                                ffier::FFIER_RESULT_SUCCESS
                            }
                        }
                    }
                    None if builder.is_some_and(|b| b.is_by_value) => {
                        let struct_path = builder.unwrap().struct_path;
                        quote! {
                            Ok(new_self) => {
                                let __new_ptr = <#struct_path as #lib_crate::FfiType>::into_c(new_self);
                                unsafe { *__handle_slot = __new_ptr };
                                ffier::FFIER_RESULT_SUCCESS
                            }
                        }
                    }
                    None => quote! {
                        Ok(_) => ffier::FFIER_RESULT_SUCCESS,
                    },
                };

                let err_branch = if builder.is_some_and(|b| b.is_by_value) {
                    quote! {
                        Err(e) => {
                            let __r = ffier::ffier_result(#err_type_tag, #lib_crate::FfiError::code(&e));
                            unsafe { *__handle_slot = core::ptr::null_mut() };
                            #box_expr
                            __r
                        }
                    }
                } else {
                    quote! {
                        Err(e) => {
                            let __r = ffier::ffier_result(#err_type_tag, #lib_crate::FfiError::code(&e));
                            #box_expr
                            __r
                        }
                    }
                };

                quote! {
                    match #call_expr {
                        #ok_branch
                        #err_branch
                    }
                }
            }
        }
    }
}
