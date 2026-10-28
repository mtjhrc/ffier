//! Export direct public functions while their cfg-selected siblings are visible.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, Item, ItemFn, ItemMod, Visibility};

use crate::{
    AliasContext, ExportCfg, MACRO_COUNTER, MethodMetaKind, combine_cfg_predicates,
    emit_method_meta, extract_cfg_predicates, parse_ffier_method_attrs, parse_method_sig,
    reexport_helpers, strip_ffier_attrs,
};

pub fn export(mut module: ItemMod, config: &ExportCfg) -> syn::Result<TokenStream> {
    if config.cfg.is_some() {
        return Err(syn::Error::new_spanned(
            &module,
            "ffier module exports use #[cfg(...)] on their functions instead of export(cfg = ...)",
        ));
    }
    let Some((_, items)) = &mut module.content else {
        return Err(syn::Error::new_spanned(
            module,
            "ffier module exports require an inline module",
        ));
    };
    let mut groups: Vec<Vec<ItemFn>> = Vec::new();
    for item in items.iter_mut() {
        let Item::Fn(function) = item else { continue };
        if !matches!(function.vis, Visibility::Public(_)) {
            continue;
        }
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group[0].sig.ident == function.sig.ident)
        {
            group.push(function.clone());
        } else {
            groups.push(vec![function.clone()]);
        }
        strip_ffier_attrs(&mut function.attrs);
        for arg in &mut function.sig.inputs {
            if let FnArg::Typed(param) = arg {
                strip_ffier_attrs(&mut param.attrs);
            }
        }
    }
    let generated = groups
        .iter()
        .map(|group| export_group(group))
        .collect::<syn::Result<Vec<_>>>()?;
    for tokens in generated {
        items.push(Item::Verbatim(tokens));
    }
    Ok(quote! { #module })
}

fn export_group(functions: &[ItemFn]) -> syn::Result<TokenStream> {
    let name = &functions[0].sig.ident;
    let name_string = name.to_string();
    let helper = format_ident!("_ffier_fn_{name_string}");
    let mut variants = Vec::new();
    let mut methods = Vec::new();
    for (index, function) in functions.iter().enumerate() {
        let variant = format_ident!("__variant_{index}");
        let mut context = AliasContext::new(helper.clone());
        context.helper_mod = quote! { #helper::#variant };
        let attrs = parse_ffier_method_attrs(&function.attrs)?;
        if attrs.foreign_return.is_some() && attrs.foreign_return_c_name.is_none() {
            return Err(syn::Error::new_spanned(
                &function.sig,
                "#[ffier(foreign_return = ...)] requires c_name = \"...\" (the C typedef name for the foreign handle type)",
            ));
        }
        let mut method = parse_method_sig(
            &function.sig,
            &function.attrs,
            &mut context,
            None,
            false,
            false,
            attrs.foreign_return.as_ref(),
            attrs.foreign_return_c_name.as_deref(),
        )?
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &function.sig,
                "ffier: could not parse free function signature",
            )
        })?;
        method.ffi_name = name_string.clone();
        methods.push(method);
        let predicate = combine_cfg_predicates(&extract_cfg_predicates(&function.attrs)?);
        let cfg = crate::meta::cfg_predicate_attr(predicate.as_ref());
        let aliases = context.local_type_aliases();
        variants.push(quote! {
            #cfg
            #[doc(hidden)]
            pub mod #variant {
                #(#aliases)*
            }
        });
    }
    let predicates: Vec<_> = methods
        .iter()
        .filter_map(|m| combine_cfg_predicates(&m.cfg_predicates))
        .collect();
    let available = (predicates.len() == methods.len()).then(|| quote! { any(#(#predicates),*) });
    let available = crate::meta::cfg_predicate_attr(available.as_ref());
    let methods = emit_method_meta(&methods, MethodMetaKind::Impl);
    let counter = MACRO_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let internal = format_ident!("__ffier_internal_fn_{name_string}_{counter}");
    let metadata = format_ident!("__ffier_meta_{name}");
    let (reexports, bundle) = reexport_helpers(&[&helper], &quote! {});
    Ok(quote! {
        #bundle
        #[doc(hidden)]
        pub mod #helper {
            #[allow(unused_imports)]
            use super::*;
            #available
            pub use super::#name as __function;
            #(#variants)*
        }
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal {
            #reexports
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exported_fn,
                    name = #name,
                    fn_path = ($crate::#helper::__function),
                    prefix = $prefix,
                    ffi_name = #name_string,
                    methods = [#(#methods),*],
                } $(, $($rest)*)? }
            };
        }
        #[doc(hidden)]
        pub use #internal as #metadata;
    })
}
