//! Parameter marshaling and callback conversion for bridge calls.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use crate::meta::{MetaParam, MetaParamKind, cfg_predicate_attr};

use super::dispatch::TraitMap;
use super::signature::CExternSignature;

/// Info about a single `impl Trait` param, used for dispatch codegen.
pub(super) struct ImplTraitParam {
    pub(super) name: syn::Ident,
    dispatch: crate::meta::DispatchMode,
    pub(super) ref_kind: crate::meta::ImplTraitRefKind,
    pub(super) trait_name: String,
    trait_lifetime_args: Vec<syn::Ident>,
    lifetime_bounds: Vec<syn::Ident>,
    /// (name, bridge_type, cfg_predicate) for each concrete dispatch variant.
    pub(super) variants: Vec<(String, TokenStream2, Option<TokenStream2>)>,
}

/// Intermediate result of converting method/function params into bridge code.
pub(super) struct ConvertedParams {
    /// Pre-binding statements (e.g. `let __tags_vec = { ... };`)
    pub(super) pre_bindings: Vec<TokenStream2>,
    /// Vtable dispatch pre-bindings (e.g. `let mut __dyn_box_fruit = ...;`)
    pub(super) vtable_pre_bindings: Vec<TokenStream2>,
    /// Converted argument expressions to pass to the Rust function call.
    pub(super) converted_args: Vec<TokenStream2>,
    /// Impl Trait params with resolved effective dispatch — concrete params
    /// (effective_dispatch = false) need nested type-tag matching via
    /// `wrap_concrete_dispatch`.
    pub(super) concrete_dispatch_params: Vec<ImplTraitParam>,
}

/// Build the body pieces shared between method and free function bridge
/// generation: param conversion, pre-bindings, and impl Trait dispatch.
///
/// `c_sig` is used to look up the `_len` ident for StrSlice params.
/// `ffi_name_str` is the FFI function name (for error messages in dispatch panics).
pub(super) fn convert_params(
    params: &[MetaParam],
    c_sig: &CExternSignature,
    ffi_name_str: &str,
    trait_map: &TraitMap,
    handle_types: &std::collections::HashSet<String>,
    lib_crate: &TokenStream2,
) -> Result<ConvertedParams, TokenStream2> {
    for p in params {
        if let MetaParamKind::ImplTrait {
            ref_kind,
            ref_lifetime: Some(lifetime),
            ..
        } = &p.kind
            && *ref_kind != crate::meta::ImplTraitRefKind::Value
            && lifetime != "_"
        {
            let msg = format!(
                "ffier: borrowed callback `{}` with explicit reference lifetime '{lifetime} is unsupported; callback handles are call-scoped",
                p.name
            );
            return Err(quote! { compile_error!(#msg); });
        }
    }
    // Collect all impl Trait params with their dispatch info.
    // If a trait has no entry in trait_map (no concrete implementors),
    // the param still needs vtable dispatch (empty variants list forces
    // vtable mode since concrete dispatch with 0 variants makes no sense).
    let impl_trait_params: Vec<_> = params
        .iter()
        .filter_map(|p| {
            if let MetaParamKind::ImplTrait {
                trait_name,
                dispatch,
                ref_kind,
                trait_lifetime_args,
                lifetime_bounds,
                ..
            } = &p.kind
            {
                let info = trait_map.get(trait_name);
                Some(ImplTraitParam {
                    name: p.name.clone(),
                    dispatch: *dispatch,
                    ref_kind: *ref_kind,
                    trait_name: trait_name.clone(),
                    trait_lifetime_args: trait_lifetime_args.clone(),
                    lifetime_bounds: lifetime_bounds.clone(),
                    variants: info
                        .map(|i| {
                            i.variants
                                .iter()
                                .map(|v| {
                                    (
                                        v.name.clone(),
                                        v.bridge_type.clone(),
                                        v.cfg_predicate.clone(),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            } else {
                None
            }
        })
        .collect();

    // Check for dispatch limit (auto mode only)
    let concrete_params: Vec<_> = impl_trait_params
        .iter()
        .filter(|p| p.dispatch != crate::meta::DispatchMode::Vtable)
        .collect();
    let total_branches: u64 = concrete_params
        .iter()
        .map(|p| p.variants.len() as u64)
        .product();
    if total_branches > crate::meta::DEFAULT_MAX_DISPATCH
        && impl_trait_params
            .iter()
            .any(|p| p.dispatch == crate::meta::DispatchMode::Auto)
    {
        let msg = format!(
            "ffier: `{ffi_name_str}` would generate {total_branches} dispatch \
             branches (limit: {}). Add `#[ffier(dispatch = vtable)]` to the impl Trait \
             param(s) or `#[ffier(dispatch = concrete)]` to override the limit.",
            crate::meta::DEFAULT_MAX_DISPATCH,
        );
        return Err(quote! { compile_error!(#msg); });
    }

    // Check vtable dispatch is possible (trait must be exported)
    for p in &impl_trait_params {
        if p.dispatch == crate::meta::DispatchMode::Vtable
            && trait_map
                .get(&p.trait_name)
                .and_then(|info| info.implementable.as_ref())
                .is_none()
        {
            let msg = format!(
                "ffier: `#[ffier(dispatch = vtable)]` on param `{}` requires trait `{}` \
                 to have `#[ffier::export]`",
                p.name, p.trait_name,
            );
            return Err(quote! { compile_error!(#msg); });
        }
    }

    // Convert params
    let local_token = crate::ffi_traits::token_of(lib_crate);
    let mut pre_bindings = Vec::new();
    let mut converted_args = Vec::new();
    for p in params {
        let len_name = format!("{}_len", p.name);
        let len = c_sig
            .params
            .iter()
            .find(|cp| cp.name == len_name)
            .map(|cp| &cp.name);
        let converted = crate::conv::lift_param(p, len, handle_types, &local_token);
        pre_bindings.extend(converted.pre);
        converted_args.extend(converted.exprs);
    }

    // Determine effective dispatch mode for each param.
    // When total_branches == 0 (no concrete implementors), force vtable
    // dispatch — there's nothing to dispatch concretely against.
    let all_concrete = total_branches > 0
        && impl_trait_params
            .iter()
            .all(|p| p.dispatch != crate::meta::DispatchMode::Vtable)
        && (total_branches <= crate::meta::DEFAULT_MAX_DISPATCH
            || impl_trait_params
                .iter()
                .all(|p| p.dispatch == crate::meta::DispatchMode::Concrete));
    let effective_dispatch: Vec<bool> = if all_concrete {
        vec![false; impl_trait_params.len()]
    } else {
        let mut first_auto_seen = false;
        impl_trait_params
            .iter()
            .map(|p| match p.dispatch {
                crate::meta::DispatchMode::Concrete => false,
                crate::meta::DispatchMode::Vtable => true,
                crate::meta::DispatchMode::Auto => {
                    if !first_auto_seen {
                        first_auto_seen = true;
                        false
                    } else {
                        true
                    }
                }
            })
            .collect()
    };

    // Dynamic dispatch: keep owned values owned and borrow references directly.
    // For &/&mut params, borrow the handle directly.
    let mut vtable_pre_bindings: Vec<TokenStream2> = Vec::new();
    for (i, p) in impl_trait_params.iter().enumerate() {
        if !effective_dispatch[i] {
            continue;
        }
        let dyn_id = &p.name;
        let info = trait_map.get(&p.trait_name).unwrap();
        let trait_ident = if let Some(imp) = &info.implementable {
            imp.trait_path.clone()
        } else {
            let ident = format_ident!("{}", p.trait_name);
            quote! { #ident }
        };
        // C erases lifetimes. Safe clients enforce original lifetimes before
        // crossing this unsafe boundary; trait arguments are not value bounds.
        let trait_args: Vec<_> = p
            .trait_lifetime_args
            .iter()
            .map(|_| quote! { 'static })
            .collect();
        let trait_ident = if trait_args.is_empty() {
            trait_ident
        } else {
            quote! { #trait_ident <#(#trait_args),*> }
        };
        let object_lifetime = if p.lifetime_bounds.iter().any(|lt| lt == "static") {
            quote! { 'static }
        } else {
            quote! { '_ }
        };

        let expected_msg = format!("impl {}", p.trait_name);
        let accepted_const = format_ident!("__FFIER_ACCEPTED_{}", p.trait_name);

        use crate::meta::ImplTraitRefKind;
        match p.ref_kind {
            ImplTraitRefKind::Value => {
                let mut branches = Vec::new();
                for v in &info.variants {
                    let ty = &v.bridge_type;
                    let vcfg = cfg_predicate_attr(v.cfg_predicate.as_ref());
                    branches.push(quote! {
                        #vcfg
                        if __type_tag == <#ty as ffier::FfiHandle<#lib_crate::__FfierLocalToken>>::TYPE_TAG {
                            let __val = unsafe { ffier::ffier_handle_consume::<#ty>(#dyn_id, <#ty as ffier::FfiHandle<#lib_crate::__FfierLocalToken>>::TYPE_TAG) };
                            break 'dispatch (Box::new(__val) as Box<dyn #trait_ident + #object_lifetime>);
                        }
                    });
                }
                vtable_pre_bindings.push(quote! {
                    let #dyn_id: Box<dyn #trait_ident + #object_lifetime> = 'dispatch: {
                        let __type_tag = unsafe { __ffier_handle_tag(#ffi_name_str, #dyn_id) };
                        #(#branches)*
                        __ffier_dispatch_panic(#ffi_name_str, #expected_msg, #accepted_const, __type_tag);
                    };
                });
            }
            ImplTraitRefKind::Mut => {
                let borrow_id = format_ident!("__borrow_mut_{}", p.name);
                let mut branches = Vec::new();
                for v in &info.variants {
                    let ty = &v.bridge_type;
                    let vcfg = cfg_predicate_attr(v.cfg_predicate.as_ref());
                    branches.push(quote! {
                        #vcfg
                        if __type_tag == <#ty as ffier::FfiHandle<#lib_crate::__FfierLocalToken>>::TYPE_TAG {
                            break 'dispatch (unsafe { ffier::ffier_handle_borrow_mut::<#ty>(#dyn_id, <#ty as ffier::FfiHandle<#lib_crate::__FfierLocalToken>>::TYPE_TAG) as &mut dyn #trait_ident });
                        }
                    });
                }
                vtable_pre_bindings.push(quote! {
                    let #borrow_id: &mut dyn #trait_ident = 'dispatch: {
                        let __type_tag = unsafe { __ffier_handle_tag(#ffi_name_str, #dyn_id) };
                        #(#branches)*
                        __ffier_dispatch_panic(#ffi_name_str, #expected_msg, #accepted_const, __type_tag);
                    };
                    let mut #borrow_id = #borrow_id;
                    let #dyn_id = &mut #borrow_id;
                });
            }
            ImplTraitRefKind::Ref => {
                let borrow_id = format_ident!("__borrow_{}", p.name);
                let mut branches = Vec::new();
                for v in &info.variants {
                    let ty = &v.bridge_type;
                    let vcfg = cfg_predicate_attr(v.cfg_predicate.as_ref());
                    branches.push(quote! {
                        #vcfg
                        if __type_tag == <#ty as ffier::FfiHandle<#lib_crate::__FfierLocalToken>>::TYPE_TAG {
                            break 'dispatch (unsafe { ffier::ffier_handle_borrow::<#ty>(#dyn_id, <#ty as ffier::FfiHandle<#lib_crate::__FfierLocalToken>>::TYPE_TAG) as &dyn #trait_ident });
                        }
                    });
                }
                vtable_pre_bindings.push(quote! {
                    let #borrow_id: &dyn #trait_ident = 'dispatch: {
                        let __type_tag = unsafe { __ffier_handle_tag(#ffi_name_str, #dyn_id) };
                        #(#branches)*
                        __ffier_dispatch_panic(#ffi_name_str, #expected_msg, #accepted_const, __type_tag);
                    };
                    let #dyn_id = &#borrow_id;
                });
            }
        }
    }

    // Collect concrete dispatch params (effective_dispatch = false)
    let concrete_dispatch_params: Vec<_> = impl_trait_params
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !effective_dispatch[*i])
        .map(|(_, p)| p)
        .collect();

    Ok(ConvertedParams {
        pre_bindings,
        vtable_pre_bindings,
        converted_args,
        concrete_dispatch_params,
    })
}

/// Emit a `let obj = ...` binding that borrows a value from a handle.
///
/// - `mutable = false`: `let obj = ffier::ffier_handle_borrow::<T>(handle, tag);`
/// - `mutable = true`:  `let obj = ffier::ffier_handle_borrow_mut::<T>(handle, tag);`
pub(super) fn borrow_from_handle(
    ty: &TokenStream2,
    mutable: bool,
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    let tag = quote! { <#ty as ffier::FfiHandle<#lib_crate::__FfierLocalToken>>::TYPE_TAG };
    if mutable {
        quote! {
            let obj = unsafe { ffier::ffier_handle_borrow_mut::<#ty>(handle, #tag) };
        }
    } else {
        quote! {
            let obj = unsafe { ffier::ffier_handle_borrow::<#ty>(handle, #tag) };
        }
    }
}
