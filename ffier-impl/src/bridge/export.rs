//! Emit bridges for exported types, free functions, and trait implementations.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use crate::meta::{
    HasPrefix, MetaExportable, MetaFreeFunction, MetaImplementable, MetaReceiver, MetaTraitImpl,
    camel_to_snake,
};

use super::dispatch::{ErrorMap, TraitMap, wrap_concrete_dispatch};
use super::params::{borrow_from_handle, convert_params};
use super::returns::{BuilderCtx, wrap_return};
use super::signature::c_signature_for_method;

pub(super) fn generate_exportable_bridge(
    meta: MetaExportable,
    trait_map: &TraitMap,
    error_map: &ErrorMap,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    let struct_path = &meta.struct_path;
    let struct_name = &meta.struct_name.to_string();
    let fn_pfx = meta.fn_pfx();

    let struct_lower = camel_to_snake(struct_name);

    // Whether this type has any builder methods with by-value self.
    // When true, ALL by-value self methods receive a pointer-to-handle
    // (void**) in the C ABI, not just the builder methods themselves.
    let is_builder_type = meta
        .methods
        .iter()
        .any(|m| m.is_builder() && m.receiver == MetaReceiver::Value);

    let mut ffi_fns = Vec::new();

    // Method FFI functions
    for m in &meta.methods {
        let ffi_name_str = format!("{}{}", fn_pfx, m.ffi_name());
        let ffi_name = format_ident!("{}", ffi_name_str);
        let method_name = &m.name;

        let has_receiver = m.receiver != MetaReceiver::None;
        let is_mut = m.receiver == MetaReceiver::Mut;
        let is_by_value = m.receiver == MetaReceiver::Value;
        let is_builder = m.is_builder();

        // Single source of truth: the extern "C" fn signature.
        let c_sig = c_signature_for_method(m, &meta.prefix, handle_types, value_types, lib_crate);

        // Self access via borrow/consume (instance methods only).
        //
        // For builder by-value methods, the C param is a pointer-to-pointer
        // (FtConfig* = void**). We dereference it first, storing the slot
        // address for write-back later.
        let obj_binding = if has_receiver {
            let type_assert = quote! {
                let __actual = unsafe { ffier::handle_type_tag(handle) };
                let __expected = <#struct_path as #lib_crate::FfiHandle>::TYPE_TAG;
                if __actual != __expected {
                    __ffier_dispatch_panic(
                        #ffi_name_str,
                        <#struct_path as #lib_crate::FfiHandle>::C_HANDLE_NAME,
                        "",
                        __actual,
                    );
                }
            };
            let deref_slot = if is_builder_type && is_by_value {
                // `handle` param is *mut c_void but actually void**.
                // All by-value self methods on builder types use pointer-to-handle
                // in the C ABI (so the caller can pass &builder consistently).
                quote! {
                    let __handle_slot = handle as *mut *mut core::ffi::c_void;
                    let handle = unsafe { *__handle_slot };
                }
            } else {
                quote! {}
            };
            let cast = if is_by_value {
                quote! {
                    ffier::ffier_handle_consume::<#struct_path>(handle)
                }
            } else if is_mut {
                quote! {
                    ffier::ffier_handle_borrow_mut::<#struct_path>(handle)
                }
            } else {
                quote! {
                    ffier::ffier_handle_borrow::<#struct_path>(handle)
                }
            };
            Some(quote! { #deref_slot #type_assert let obj = unsafe { #cast }; })
        } else {
            None
        };

        // Shared param conversion + impl Trait dispatch.
        let cp = match convert_params(&m.params, &c_sig, &ffi_name_str, trait_map, lib_crate) {
            Ok(cp) => cp,
            Err(err) => return err,
        };
        let converted_args = &cp.converted_args;
        let pre_bindings = &cp.pre_bindings;
        let vtable_pre_bindings = &cp.vtable_pre_bindings;

        // Build the method call expression
        let base_method_call = if has_receiver {
            quote! { obj.#method_name(#(#converted_args),*) }
        } else {
            quote! { <#struct_path>::#method_name(#(#converted_args),*) }
        };

        let method_call = wrap_concrete_dispatch(
            base_method_call,
            &cp.concrete_dispatch_params,
            &ffi_name_str,
            lib_crate,
        );

        // Extern fn signature from c_sig (shared across all return variants)
        let sig_names: Vec<_> = c_sig.params.iter().map(|p| &p.name).collect();
        let sig_types: Vec<_> = c_sig.params.iter().map(|p| &p.c_type).collect();
        let sig_ret = &c_sig.ret;

        let builder_ctx = if is_builder {
            Some(BuilderCtx {
                struct_path,
                is_by_value,
            })
        } else {
            None
        };
        let return_body = wrap_return(
            method_call,
            &m.ret,
            &m.rust_ret,
            handle_types,
            value_types,
            error_map,
            builder_ctx.as_ref(),
            lib_crate,
        );

        let cfg_attr = m.cfg_attr();
        ffi_fns.push(quote! {
            #cfg_attr
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn #ffi_name(
                #(#sig_names: #sig_types),*
            ) #sig_ret {
                #obj_binding
                #(#vtable_pre_bindings)*
                #(#pre_bindings)*
                #return_body
            }
        });
    }

    // destroy function
    let destroy_name = format_ident!("{fn_pfx}{struct_lower}_destroy");
    let destroy_str = destroy_name.to_string();

    ffi_fns.push(quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #destroy_name(handle: *mut core::ffi::c_void) {
            if !handle.is_null() {
                let __actual = unsafe { ffier::handle_type_tag(handle) };
                let __expected = <#struct_path as #lib_crate::FfiHandle>::TYPE_TAG;
                if __actual != __expected {
                    __ffier_dispatch_panic(
                        #destroy_str,
                        <#struct_path as #lib_crate::FfiHandle>::C_HANDLE_NAME,
                        "",
                        __actual,
                    );
                }
                unsafe { ffier::ffier_handle_drop::<#struct_path>(handle) };
            }
        }
    });

    quote! {
        #(#ffi_fns)*
    }
}

pub(super) fn generate_free_fn_bridge(
    meta: MetaFreeFunction,
    error_map: &ErrorMap,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    trait_map: &TraitMap,
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    let fn_path = &meta.fn_path;
    let fn_pfx = meta.fn_pfx();

    meta.methods
        .iter()
        .map(|m| {
            let ffi_name_str = format!("{}{}", fn_pfx, meta.ffi_name);
            let ffi_name = format_ident!("{}", ffi_name_str);

            // Use the same signature builder as methods.
            let c_sig =
                c_signature_for_method(m, &meta.prefix, handle_types, value_types, lib_crate);

            // Shared param conversion + impl Trait dispatch.
            let cp = match convert_params(&m.params, &c_sig, &ffi_name_str, trait_map, lib_crate) {
                Ok(cp) => cp,
                Err(err) => return err,
            };

            let converted_args = &cp.converted_args;
            let base_method_call = quote! { #fn_path(#(#converted_args),*) };
            let method_call = wrap_concrete_dispatch(
                base_method_call,
                &cp.concrete_dispatch_params,
                &ffi_name_str,
                lib_crate,
            );

            let sig_names: Vec<_> = c_sig.params.iter().map(|p| &p.name).collect();
            let sig_types: Vec<_> = c_sig.params.iter().map(|p| &p.c_type).collect();
            let sig_ret = &c_sig.ret;
            let pre_bindings = &cp.pre_bindings;
            let vtable_pre_bindings = &cp.vtable_pre_bindings;

            let return_body = wrap_return(
                method_call,
                &m.ret,
                &m.rust_ret,
                handle_types,
                value_types,
                error_map,
                None,
                lib_crate,
            );

            let cfg_attr = m.cfg_attr();
            quote! {
                #cfg_attr
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn #ffi_name(
                    #(#sig_names: #sig_types),*
                ) #sig_ret {
                    #(#vtable_pre_bindings)*
                    #(#pre_bindings)*
                    #return_body
                }
            }
        })
        .collect()
}

pub(super) fn generate_implementable_bridge(
    _meta: MetaImplementable,
    _lib_crate: &TokenStream2,
) -> TokenStream2 {
    // No per-type bridge functions needed for exported traits.
    // The vtable ABI is defined by the vtable struct layout, not by generated bridge code.
    quote! {}
}

pub(super) fn generate_trait_impl_bridge(
    meta: MetaTraitImpl,
    trait_map: &TraitMap,
    error_map: &ErrorMap,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    let struct_path = &meta.struct_path;
    let trait_path = &meta.trait_path;
    let fn_pfx = meta.fn_pfx();
    let struct_name_str = meta.struct_name.to_string();
    let struct_snake = camel_to_snake(&struct_name_str);
    let trait_name_str = meta.trait_name.to_string();
    let trait_snake = camel_to_snake(&trait_name_str);

    // When struct and trait have the same snake_case name (e.g. Error for Error),
    // the per-impl bridge functions would collide with the trait dispatch functions.
    // Skip generating bridge functions — dispatch already handles this case.
    if struct_snake == trait_snake {
        return quote! {};
    }

    let mut bridge_fns = Vec::new();

    for m in &meta.methods {
        let method_name = &m.name;
        let ffi_name_str = format!("{fn_pfx}{struct_snake}_{method_name}");
        let ffi_name = format_ident!("{ffi_name_str}");

        // Use shared signature builder + return wrapper
        let c_sig = c_signature_for_method(m, &meta.prefix, handle_types, value_types, lib_crate);
        let sig_names: Vec<_> = c_sig.params.iter().map(|p| &p.name).collect();
        let sig_types: Vec<_> = c_sig.params.iter().map(|p| &p.c_type).collect();
        let sig_ret = &c_sig.ret;

        // Shared param conversion
        let cp = match convert_params(&m.params, &c_sig, &ffi_name_str, trait_map, lib_crate) {
            Ok(cp) => cp,
            Err(err) => return err,
        };
        let converted_args = &cp.converted_args;

        let base_call = if m.raw_handle() {
            quote! {
                <#struct_path as #trait_path>::#method_name(
                    handle as *const ffier::FfierHandle<#struct_path> #(, #converted_args)*)
            }
        } else {
            let borrow = borrow_from_handle(&quote! { #struct_path }, m.is_mut());
            quote! { { #borrow <#struct_path as #trait_path>::#method_name(obj, #(#converted_args),*) } }
        };
        let call_expr = wrap_concrete_dispatch(
            base_call,
            &cp.concrete_dispatch_params,
            &ffi_name_str,
            lib_crate,
        );

        let return_body = wrap_return(
            call_expr,
            &m.ret,
            &m.rust_ret,
            handle_types,
            value_types,
            error_map,
            None,
            lib_crate,
        );

        let pre_bindings = &cp.pre_bindings;
        let vtable_pre_bindings = &cp.vtable_pre_bindings;
        let cfg_attr = m.cfg_attr();
        bridge_fns.push(quote! {
            #cfg_attr
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn #ffi_name(#(#sig_names: #sig_types),*) #sig_ret {
                #(#vtable_pre_bindings)*
                #(#pre_bindings)*
                #return_body
            }
        });
    }

    quote! {
        #(#bridge_fns)*
    }
}
