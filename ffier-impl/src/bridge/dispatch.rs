//! Trait dispatch lookup, concrete parameter dispatch, and self dispatch bridges.

use std::collections::{HashMap, HashSet};

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use crate::meta::{
    MetaImplementable, MetaMethod, MetaReceiver, MetaTraitImpl, camel_to_snake, cfg_predicate_attr,
};

use super::params::{ImplTraitParam, borrow_from_handle, convert_params};
use super::returns::wrap_return;
use super::signature::c_signature_for_method;

/// Maps trait names to their concrete dispatch variants.
pub(super) type TraitMap = HashMap<String, TraitDispatchInfo>;

/// Maps error type names to their metadata (type tag, path, variants).
///
/// Used by bridge generation to emit `ffier_result(type_tag, code)`
/// with the correct type tag for `Result<T, E>` returns.
pub(super) type ErrorMap = HashMap<String, ErrorInfo>;

pub(super) struct ErrorInfo {
    pub(super) type_tag: u32,
}

pub(super) struct TraitDispatchInfo {
    pub(super) variants: Vec<TraitVariant>,
    /// If the trait has an `@implementable` metadata entry, trait-level dispatch info.
    pub(super) implementable: Option<ImplementableInfo>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TraitVariantKind {
    Concrete,
    Wrapper,
}

pub(super) struct TraitVariant {
    pub(super) name: String,
    pub(super) bridge_type: TokenStream2,
    kind: TraitVariantKind,
    /// `#[cfg(...)]` predicate. `None` = unconditional.
    pub(super) cfg_predicate: Option<TokenStream2>,
}

pub(super) struct ImplementableInfo {
    pub(super) trait_path: TokenStream2,

    methods: Vec<MetaMethod>,
    /// Number of methods that belong to this trait (not supertrait methods).
    /// Only the first `own_method_count` methods are dispatched in self-dispatch
    /// functions. Supertrait methods need separate dispatch through their own trait.
    own_method_count: usize,
    /// `#[cfg(...)]` predicate for the entire trait. `None` = unconditional.
    pub(super) cfg_predicate: Option<TokenStream2>,
}

/// Build the trait-to-impls map from parsed `@implementable` and `@trait_impl` metadata.
pub(super) fn build_trait_map(
    implementables: &[TokenStream2],
    trait_impls: &[TokenStream2],
) -> TraitMap {
    let mut map = TraitMap::new();

    // trait_impl entries: "Fruit for Apple" → Apple is a concrete implementor
    for item in trait_impls {
        if let Ok(meta) = syn::parse2::<MetaTraitImpl>(item.clone()) {
            let trait_name = meta.trait_name.to_string();
            let struct_name = meta.struct_name.to_string();
            let struct_path = meta.struct_path;
            let cfg_predicate = meta.cfg_predicate;
            map.entry(trait_name)
                .or_insert_with(|| TraitDispatchInfo {
                    variants: Vec::new(),
                    implementable: None,
                })
                .variants
                .push(TraitVariant {
                    name: struct_name,
                    bridge_type: struct_path,
                    kind: TraitVariantKind::Concrete,
                    cfg_predicate,
                });
        }
    }

    // implementable entries: "trait Fruit" → adds VtableFruit wrapper + stores vtable info
    for item in implementables {
        if let Ok(meta) = syn::parse2::<MetaImplementable>(item.clone()) {
            let trait_name = meta.trait_name.to_string();
            let wrapper_name = format!("Vtable{trait_name}");
            let wrapper_path = meta.wrapper_name.clone();
            let methods = meta.methods;
            let own_method_count = meta.own_method_count;
            let no_vtable = meta.no_vtable;

            let info = map.entry(trait_name).or_insert_with(|| TraitDispatchInfo {
                variants: Vec::new(),
                implementable: None,
            });
            // no_vtable traits have no VtableWrapper type — C callers can't
            // implement them, only concrete Rust types are dispatched.
            if !no_vtable {
                info.variants.push(TraitVariant {
                    name: wrapper_name,
                    bridge_type: wrapper_path.clone(),
                    kind: TraitVariantKind::Wrapper,
                    cfg_predicate: meta.cfg_predicate.clone(),
                });
            }
            info.implementable = Some(ImplementableInfo {
                trait_path: meta.trait_path,
                methods,
                own_method_count,
                cfg_predicate: meta.cfg_predicate,
            });
        }
    }

    map
}

/// Wrap a base call expression in concrete impl Trait dispatch (nested
/// type-tag matches for non-vtable impl Trait params).
pub(super) fn wrap_concrete_dispatch(
    base_call: TokenStream2,
    concrete_params: &[ImplTraitParam],
    ffi_name_str: &str,
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    concrete_params
        .iter()
        .rev()
        .fold(base_call, |inner, p| {
            let dyn_id = &p.name;
            let variants = &p.variants;
            let label = syn::Lifetime::new(
                &format!("'__dispatch_{}", p.name),
                proc_macro2::Span::call_site(),
            );
            use crate::meta::ImplTraitRefKind;
            let if_branches: Vec<_> = variants
                .iter()
                .map(|(_, ty_tokens, vcfg)| {
                    let vcfg_attr = cfg_predicate_attr(vcfg.as_ref());
                    let binding = match p.ref_kind {
                        ImplTraitRefKind::Value => quote! {
                            let #dyn_id = unsafe { ffier::ffier_handle_consume::<#ty_tokens>(#dyn_id) };
                        },
                        ImplTraitRefKind::Mut => quote! {
                            let #dyn_id = unsafe { ffier::ffier_handle_borrow_mut::<#ty_tokens>(#dyn_id) };
                        },
                        ImplTraitRefKind::Ref => quote! {
                            let #dyn_id = unsafe { ffier::ffier_handle_borrow::<#ty_tokens>(#dyn_id) };
                        },
                    };
                    quote! {
                        #vcfg_attr
                        if __type_tag == <#ty_tokens as #lib_crate::FfiHandle>::TYPE_TAG {
                            #binding
                            break #label { #inner };
                        }
                    }
                })
                .collect();

            let expected_msg = format!("impl {}", p.trait_name);
            let accepted_const = format_ident!("__FFIER_ACCEPTED_{}", p.trait_name);

            quote! { #label: {
                let __type_tag = unsafe { ffier::handle_type_tag(#dyn_id) };
                #(#if_branches)*
                __ffier_dispatch_panic(#ffi_name_str, #expected_msg, #accepted_const, __type_tag);
            }}
        })
}

/// Generate per-trait dispatching C functions for an exported trait.
///
/// For each method on the trait, generates a single C function that reads the
/// type tag from the handle and dispatches to the concrete implementor's method.
/// Also generates a dispatching destroy function.
///
/// Example: for `trait Fruit` with method `value(&self) -> i32` and
/// implementors `Apple`, `Orange`, `VtableFruit`:
///
/// ```c
/// int32_t ft_fruit_value(void* handle);
/// void ft_fruit_destroy(void* handle);
/// ```
#[allow(clippy::too_many_arguments)]
pub(super) fn generate_self_dispatch_bridge(
    trait_name: &str,
    info: &TraitDispatchInfo,
    prefix: &str,
    trait_map: &TraitMap,
    error_map: &ErrorMap,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    let imp = info
        .implementable
        .as_ref()
        .expect("generate_self_dispatch_bridge called for non-exported trait");
    let trait_path = &imp.trait_path;
    let trait_snake = camel_to_snake(trait_name);
    let fn_pfx = format!("{prefix}_");

    let accepted_const = format_ident!("__FFIER_ACCEPTED_{trait_name}");

    let mut bridge_fns = Vec::new();

    // Generate dispatching functions for each trait method (own methods only,
    // not supertrait methods — those need their own dispatch via their own trait).
    // TODO: Reconsider the `supers(...)` syntax on #[ffier::export] for traits.
    //       Perhaps supertrait methods should be exported by making the supertrait
    //       itself `#[ffier::export]` (or at least having its own trait_impl
    //       entries), rather than inlining them into the subtrait's vtable.
    let own_methods = &imp.methods[..imp.own_method_count];
    for m in own_methods.iter() {
        let method_name = &m.name;
        let ffi_name_str = format!("{fn_pfx}{trait_snake}_{method_name}");
        let ffi_name = format_ident!("{ffi_name_str}");

        // Use the shared signature builder for params + return type.
        // Self-dispatch always needs a `handle` param (even for raw_handle
        // methods that have MetaReceiver::None) because the dispatcher reads
        // the type tag from it. c_signature_for_method only adds handle for
        // methods with a receiver, so prepend it when missing.
        let c_sig = c_signature_for_method(m, prefix, handle_types, value_types, lib_crate);
        let has_receiver = m.receiver != MetaReceiver::None;
        let mut all_params: Vec<(&syn::Ident, &TokenStream2)> = Vec::new();
        let handle_name = format_ident!("handle");
        let handle_type = quote! { *mut core::ffi::c_void };
        if !has_receiver {
            all_params.push((&handle_name, &handle_type));
        }
        for p in &c_sig.params {
            all_params.push((&p.name, &p.c_type));
        }
        let sig_names: Vec<_> = all_params.iter().map(|(n, _)| *n).collect();
        let sig_types: Vec<_> = all_params.iter().map(|(_, t)| *t).collect();
        let sig_ret = &c_sig.ret;

        // Shared param conversion (same as exported methods / free functions).
        let cp = match convert_params(&m.params, &c_sig, &ffi_name_str, trait_map, lib_crate) {
            Ok(cp) => cp,
            Err(err) => return err,
        };
        let converted_args = &cp.converted_args;
        let pre_bindings = &cp.pre_bindings;
        let vtable_pre_bindings = &cp.vtable_pre_bindings;

        // Vtable indices are u16: the header stores its slot count as u16.
        let method_index_u16 =
            u16::try_from(m.index()).expect("vtable index must fit in u16 (checked at parse time)");

        // Default helper path for defaulted methods.
        let default_helper_path = if m.has_default() {
            let helper_ident = format_ident!("__ffier_default_{}_{}", trait_name, method_name);
            let mut tokens: Vec<proc_macro2::TokenTree> = trait_path.clone().into_iter().collect();
            if let Some(last) = tokens.last_mut() {
                *last = proc_macro2::TokenTree::Ident(helper_ident);
            }
            Some(tokens.into_iter().collect::<TokenStream2>())
        } else {
            None
        };

        let dispatch_branches: Vec<_> = info
            .variants
            .iter()
            .map(|v| {
                let ty = &v.bridge_type;

                // For the VtableFoo variant of defaulted methods, check metadata
                // before calling through the vtable. Skip for raw_handle methods
                // since exported trait macros don't generate default helpers for them.
                let metadata_guard =
                    if v.kind == TraitVariantKind::Wrapper && m.has_default() && !m.raw_handle() {
                        if let Some(helper) = &default_helper_path {
                            let obj_for_default = borrow_from_handle(ty, m.is_mut());
                            let default_base_call = quote! { #helper(obj #(, #converted_args)*) };
                            let default_call_expr = wrap_concrete_dispatch(
                                default_base_call,
                                &cp.concrete_dispatch_params,
                                &ffi_name_str,
                                lib_crate,
                            );
                            let default_body = wrap_return(
                                default_call_expr,
                                &m.ret,
                                &m.rust_ret,
                                handle_types,
                                value_types,
                                error_map,
                                None,
                                lib_crate,
                            );
                            quote! {
                                let __metadata = unsafe { ffier::handle_metadata(handle) };
                                if ffier::default_dispatch_method_index(__metadata)
                                    == Some(#method_index_u16)
                                {
                                    #obj_for_default
                                    return { #default_body };
                                }
                            }
                        } else {
                            quote! {}
                        }
                    } else {
                        quote! {}
                    };

                let (base_call, pre_binding) = if m.raw_handle() {
                    (
                        quote! {
                            <#ty as #trait_path>::#method_name(
                                handle as *const ffier::FfierHandle<#ty> #(, #converted_args)*)
                        },
                        quote! {},
                    )
                } else {
                    let obj_binding = borrow_from_handle(ty, m.is_mut());
                    (
                        quote! {
                            <#ty as #trait_path>::#method_name(obj #(, #converted_args)*)
                        },
                        obj_binding,
                    )
                };
                let call_expr = wrap_concrete_dispatch(
                    base_call,
                    &cp.concrete_dispatch_params,
                    &ffi_name_str,
                    lib_crate,
                );

                let ret_body = wrap_return(
                    call_expr,
                    &m.ret,
                    &m.rust_ret,
                    handle_types,
                    value_types,
                    error_map,
                    None,
                    lib_crate,
                );

                let variant_cfg = cfg_predicate_attr(v.cfg_predicate.as_ref());
                quote! {
                    #variant_cfg
                    if __type_tag == <#ty as #lib_crate::FfiHandle>::TYPE_TAG {
                        #metadata_guard
                        #pre_binding
                        return { #ret_body };
                    }
                }
            })
            .collect();

        let expected_str = format!("{trait_name} implementor");
        let cfg_attr = m.cfg_attr();
        bridge_fns.push(quote! {
            #cfg_attr
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn #ffi_name(#(#sig_names: #sig_types),*) #sig_ret {
                #(#vtable_pre_bindings)*
                #(#pre_bindings)*
                let __type_tag = unsafe { ffier::handle_type_tag(handle) };
                #(#dispatch_branches)*
                __ffier_dispatch_panic(#ffi_name_str, #expected_str, #accepted_const, __type_tag);
            }
        });
    }

    // Generate dispatching destroy function
    let destroy_name_str = format!("{fn_pfx}{trait_snake}_destroy");
    let destroy_name = format_ident!("{destroy_name_str}");

    let destroy_branches: Vec<_> = info
        .variants
        .iter()
        .map(|v| {
            let ty = &v.bridge_type;
            let variant_cfg = cfg_predicate_attr(v.cfg_predicate.as_ref());
            quote! {
                #variant_cfg
                if __type_tag == <#ty as #lib_crate::FfiHandle>::TYPE_TAG {
                    unsafe { ffier::ffier_handle_drop::<#ty>(handle) };
                    return;
                }
            }
        })
        .collect();

    let destroy_expected = format!("{trait_name} implementor");
    bridge_fns.push(quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #destroy_name(handle: *mut core::ffi::c_void) {
            if !handle.is_null() {
                let __type_tag = unsafe { ffier::handle_type_tag(handle) };
                #(#destroy_branches)*
                __ffier_dispatch_panic(#destroy_name_str, #destroy_expected, #accepted_const, __type_tag);
            }
        }
    });

    quote! {
        #(#bridge_fns)*
    }
}
