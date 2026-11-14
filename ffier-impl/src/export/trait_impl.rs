//! Export concrete trait implementation methods.

use std::sync::atomic::Ordering;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{ImplItem, ItemImpl, Type};

use super::ExportCfg;
use crate::{
    MACRO_COUNTER,
    alias::AliasContext,
    attrs::{parse_ffier_method_attrs, strip_ffier_attrs},
    meta::camel_to_snake,
    signature::{
        MethodInfo, MethodMetaKind, ParamInfo, emit_method_meta, parse_method_sig,
        validate_callback_generics,
    },
};

pub(super) fn trait_impl_inner(input: ItemImpl, export_cfg: &ExportCfg) -> TokenStream {
    // Build the output impl block with #[ffier(skip)] attributes stripped.
    let mut clean_impl = input.clone();
    for item in &mut clean_impl.items {
        if let ImplItem::Fn(method) = item {
            strip_ffier_attrs(&mut method.attrs);
        }
    }

    // Extract trait path and name
    let Some((_, trait_path, _)) = &input.trait_ else {
        return syn::Error::new_spanned(&input, "trait_impl requires a trait impl block")
            .to_compile_error()
            .into();
    };
    let trait_last_seg = trait_path
        .segments
        .last()
        .expect("trait path must have segments");
    let trait_name = trait_last_seg.ident.clone();
    let trait_snake = camel_to_snake(&trait_name.to_string());

    // Extract lifetime arguments from the trait path (e.g. 'static from AttachDevice<'static>,
    // or 'a from AttachDevice<'a>). These may differ from the impl's declared generics.
    let trait_lt_args: Vec<String> = match &trait_last_seg.arguments {
        syn::PathArguments::AngleBracketed(ab) => ab
            .args
            .iter()
            .filter_map(|arg| {
                if let syn::GenericArgument::Lifetime(lt) = arg {
                    Some(lt.ident.to_string())
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    };

    // Extract struct type
    let Type::Path(ref struct_type_path) = *input.self_ty else {
        return syn::Error::new_spanned(&input.self_ty, "expected a named struct type")
            .to_compile_error()
            .into();
    };
    let struct_last_seg = struct_type_path
        .path
        .segments
        .last()
        .expect("expected struct name");
    let struct_ident = &struct_last_seg.ident;
    let struct_name = struct_ident.to_string();
    let struct_snake = camel_to_snake(&struct_name);

    // Extract lifetime arguments from the struct type (e.g. 'a from View<'a>).
    // Structs without lifetime params (e.g. Widget) will produce an empty list.
    let struct_lt_args: Vec<String> = match &struct_last_seg.arguments {
        syn::PathArguments::AngleBracketed(ab) => ab
            .args
            .iter()
            .filter_map(|arg| {
                if let syn::GenericArgument::Lifetime(lt) = arg {
                    Some(lt.ident.to_string())
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    };

    let helper_mod_name = format_ident!("_ffier_impl_{trait_snake}_for_{struct_snake}");
    let mut ctx = AliasContext::new(helper_mod_name.clone());

    // Extract methods, skipping any marked with #[ffier(skip)].
    let methods: Vec<MethodInfo> = {
        let mut ms = Vec::new();
        for item in &input.items {
            let ImplItem::Fn(method) = item else { continue };
            let mattrs = match parse_ffier_method_attrs(&method.attrs) {
                Ok(a) => a,
                Err(e) => return e.to_compile_error().into(),
            };
            if mattrs.skip {
                continue;
            }
            // trait_impl methods are concrete overrides, not defaults
            match parse_method_sig(
                &method.sig,
                &method.attrs,
                &mut ctx,
                None,
                false,
                mattrs.raw_handle,
                mattrs.foreign_return.as_ref(),
                mattrs.foreign_return_c_name.as_deref(),
            ) {
                Ok(Some(mut m)) => {
                    m.ffi_name = format!("{}_{}", struct_snake, method.sig.ident);
                    ms.push(m);
                }
                Ok(None) => {}
                Err(e) => return e.to_compile_error().into(),
            }
        }
        ms
    };

    let local_type_aliases = ctx.local_type_aliases();

    let method_meta = emit_method_meta(&methods, MethodMetaKind::Impl);
    if let Err(error) = validate_callback_generics(
        &input.generics,
        methods
            .iter()
            .any(|method| method.params.iter().any(ParamInfo::is_impl_trait)),
    ) {
        return error.to_compile_error().into();
    }

    let lifetime_idents: Vec<_> = input
        .generics
        .lifetimes()
        .map(|lt| format_ident!("{}", lt.lifetime.ident))
        .collect();

    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name =
        format_ident!("__ffier_internal_{trait_snake}_for_{struct_snake}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{trait_name}_for_{struct_ident}");
    let struct_path_tokens = quote! { $crate::#struct_ident };

    let cfg_pred_tokens = export_cfg.meta_tokens();
    let cfg_attr = export_cfg.cfg_attr();

    let output = quote! {
        #cfg_attr
        #clean_impl

        #cfg_attr
        #[doc(hidden)]
        pub mod #helper_mod_name {
            #(#local_type_aliases)*
        }

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            // library_definition! passes the trait path explicitly to
            // avoid bare-name collisions at the library crate root.
            ($prefix:literal, ($($tpath:tt)*), $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exported_trait_impl,
                    trait_name = #trait_name,
                    struct_name = #struct_ident,
                    struct_path = (#struct_path_tokens),
                    trait_path = ($($tpath)*),
                    prefix = $prefix,
                    lifetimes = (#(#lifetime_idents),*),
                    trait_lifetime_args = [#(#trait_lt_args),*],
                    struct_lifetime_args = [#(#struct_lt_args),*],
                    methods = [#(#method_meta),*],
                    #cfg_pred_tokens
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
    };

    output.into()
}
