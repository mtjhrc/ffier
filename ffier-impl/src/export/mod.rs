//! Dispatch exported Rust items and parse their shared export configuration.

use ffier_schema::{LifetimeVariance, ThreadSafety};
use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    Data, DeriveInput, ItemImpl, ItemStruct, ItemTrait, LitStr, Token, parse::Parse,
    parse_macro_input,
};

use crate::cfg;

mod enums;
mod function;
mod inherent;
mod module;
mod trait_impl;
mod traits;
mod value;

use enums::exportable_enum;
use function::exportable_free_fn;
use inherent::exportable_struct_impl;
use trait_impl::trait_impl_inner;
use traits::{ImplementableArgs, implementable_inner};
use value::exportable_value_struct;

pub(crate) use traits::dispatch;

pub(crate) fn export(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item2: proc_macro2::TokenStream = item.clone().into();
    let attr2: proc_macro2::TokenStream = attr.clone().into();

    // 1. Trait definition → exported trait
    if let Ok(trait_item) = syn::parse2::<ItemTrait>(item2.clone()) {
        let args = match syn::parse2::<ImplementableArgs>(attr2) {
            Ok(a) => a,
            Err(e) => return e.to_compile_error().into(),
        };
        return implementable_inner(args, trait_item);
    }

    // Parse common args (cfg = "...")
    let export_cfg = match syn::parse2::<ExportCfg>(attr2) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };

    // 2. Fixed-layout value struct.
    if let Ok(struct_item) = syn::parse2::<ItemStruct>(item2.clone()) {
        if let Err(error) = export_cfg.reject_inherent_contracts(&struct_item) {
            return error.to_compile_error().into();
        }
        return exportable_value_struct(struct_item, &export_cfg);
    }

    // 3. Enum
    if let Ok(enum_item) = syn::parse2::<DeriveInput>(item2.clone())
        && matches!(enum_item.data, Data::Enum(_))
    {
        if let Err(error) = export_cfg.reject_inherent_contracts(&enum_item) {
            return error.to_compile_error().into();
        }
        return exportable_enum(enum_item, &export_cfg);
    }

    // Inline modules keep cfg-selected function variants together in metadata.
    if let Ok(module) = syn::parse2::<syn::ItemMod>(item2.clone()) {
        if let Err(error) = export_cfg.reject_inherent_contracts(&module) {
            return error.to_compile_error().into();
        }
        return module::export(module, &export_cfg)
            .unwrap_or_else(syn::Error::into_compile_error)
            .into();
    }

    // 4. Free function
    if let Ok(fn_item) = syn::parse2::<syn::ItemFn>(item2) {
        if let Err(error) = export_cfg.reject_inherent_contracts(&fn_item) {
            return error.to_compile_error().into();
        }
        return exportable_free_fn(fn_item, &export_cfg);
    }

    // 5. Impl block — trait impl or inherent impl
    let input = parse_macro_input!(item as ItemImpl);
    if input.trait_.is_some() {
        if let Err(error) = export_cfg.reject_inherent_contracts(&input) {
            return error.to_compile_error().into();
        }
        return trait_impl_inner(input, &export_cfg);
    }
    exportable_struct_impl(input, &export_cfg)
}

/// Common export arguments accepted by all item kinds: `cfg = "predicate"`.
/// Trait definitions accept additional args (reserved, foreign, etc.) via
/// `ImplementableArgs`.
pub(crate) struct ExportCfg {
    /// `#[cfg(...)]` predicate tokens. When set, the proc macro emits the
    /// predicate as `#[cfg(PRED)]` on generated code that references the
    /// gated type, while metadata (for schema/bridge) is emitted unconditionally.
    pub(crate) cfg: Option<proc_macro2::TokenStream>,
    /// Compiler-verified Send/Sync guarantees for the exported opaque handle.
    pub(crate) thread_safety: ThreadSafety,
    pub(crate) lifetime_variance: Vec<(syn::Lifetime, LifetimeVariance)>,
}

impl Parse for ExportCfg {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut cfg = None;
        let mut thread_safety = ThreadSafety::default();
        let mut lifetime_variance = Vec::new();
        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            if key == "cfg" {
                input.parse::<Token![=]>()?;
                let lit: LitStr = input.parse()?;
                cfg = Some(cfg::normalize_tokens(
                    &lit.parse::<proc_macro2::TokenStream>()?,
                )?);
            } else if key == "send" {
                thread_safety.send = true;
            } else if key == "sync" {
                thread_safety.sync = true;
            } else if matches!(
                key.to_string().as_str(),
                "covariant" | "contravariant" | "invariant"
            ) {
                let variance = match key.to_string().as_str() {
                    "covariant" => LifetimeVariance::Covariant,
                    "contravariant" => LifetimeVariance::Contravariant,
                    _ => LifetimeVariance::Invariant,
                };
                let content;
                syn::parenthesized!(content in input);
                let lifetimes = content.parse_terminated(syn::Lifetime::parse, Token![,])?;
                if lifetimes.is_empty() {
                    return Err(syn::Error::new(
                        key.span(),
                        "ffier variance groups require at least one lifetime",
                    ));
                }
                for lifetime in lifetimes {
                    if lifetime_variance
                        .iter()
                        .any(|(previous, _)| previous == &lifetime)
                    {
                        return Err(syn::Error::new_spanned(
                            lifetime,
                            "ffier lifetime variance must be declared exactly once",
                        ));
                    }
                    lifetime_variance.push((lifetime, variance));
                }
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "unknown #[ffier::export] argument `{key}`; expected `cfg`, `send`, `sync`, `covariant`, `contravariant`, or `invariant`"
                    ),
                ));
            }
            let _ = input.parse::<Token![,]>();
        }
        Ok(ExportCfg {
            cfg,
            thread_safety,
            lifetime_variance,
        })
    }
}

impl ExportCfg {
    /// Send/sync and variance groups apply only to inherent impls.
    /// Everything else — enums, free functions, trait impls — must reject them.
    pub(crate) fn reject_inherent_contracts(&self, item: &impl ToTokens) -> syn::Result<()> {
        if !self.thread_safety.is_empty() {
            return Err(syn::Error::new_spanned(
                item,
                "ffier send/sync annotations are supported only on inherent impls; traits use native supertraits",
            ));
        }
        if !self.lifetime_variance.is_empty() {
            return Err(syn::Error::new_spanned(
                item,
                "ffier lifetime variance annotations are supported only on inherent impls",
            ));
        }
        Ok(())
    }

    /// Emit `#[cfg(PRED)]` token stream if cfg is set.
    pub(crate) fn cfg_attr(&self) -> proc_macro2::TokenStream {
        match &self.cfg {
            Some(pred) => quote! { #[cfg(#pred)] },
            None => quote! {},
        }
    }

    /// Emit `cfg_predicate = (...),` for metadata blobs if cfg is set.
    pub(crate) fn meta_tokens(&self) -> proc_macro2::TokenStream {
        match &self.cfg {
            Some(pred) => quote! { cfg_predicate = (#pred), },
            None => quote! {},
        }
    }
}
