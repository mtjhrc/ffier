//! Collect sibling free-function signatures before Rust's item-level cfg
//! filtering loses the inactive alternatives. Metadata still uses the same
//! cfg predicates and method parser as exported impls.
//!
//! This is derived from the definition's source, not an expansion-order
//! registry: separate proc-macro processes and incremental builds produce the
//! same metadata. Macro-generated functions retain the single-item path.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Item, ItemFn, Meta};

use crate::{ExportCfg, combine_cfg_predicates, extract_cfg_predicates};

pub struct Variant {
    pub function: ItemFn,
    pub predicate: Option<TokenStream>,
    invocation: Option<TokenStream>,
}

pub struct Group {
    pub variants: Vec<Variant>,
    pub emitter: Option<TokenStream>,
}

fn comma_parts(tokens: TokenStream) -> Vec<TokenStream> {
    let mut parts = vec![TokenStream::new()];
    for token in tokens {
        if matches!(&token, proc_macro2::TokenTree::Punct(p) if p.as_char() == ',') {
            parts.push(TokenStream::new());
        } else {
            parts.last_mut().unwrap().extend([token]);
        }
    }
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

fn export_args(
    meta: &Meta,
    conditions: &[TokenStream],
) -> syn::Result<Option<(ExportCfg, Vec<TokenStream>)>> {
    if meta
        .path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "export")
    {
        let cfg = match meta {
            Meta::Path(_) => ExportCfg { cfg: None },
            Meta::List(list) => syn::parse2(list.tokens.clone())?,
            _ => return Ok(None),
        };
        return Ok(Some((cfg, conditions.to_vec())));
    }
    if let Meta::List(list) = meta
        && list.path.is_ident("cfg_attr")
    {
        let parts = comma_parts(list.tokens.clone());
        if let Some(condition) = parts.first() {
            let mut nested_conditions = conditions.to_vec();
            nested_conditions.push(condition.clone());
            for item in &parts[1..] {
                if let Some(export) = export_args(&syn::parse2(item.clone())?, &nested_conditions)?
                {
                    return Ok(Some(export));
                }
            }
        }
    }
    Ok(None)
}

fn source_variant(function: &ItemFn) -> syn::Result<Option<Variant>> {
    for attr in &function.attrs {
        if let Some((cfg, mut invocation)) = export_args(&attr.meta, &[])? {
            let mut predicates = extract_cfg_predicates(&function.attrs);
            invocation.extend(predicates.iter().cloned());
            if let Some(cfg) = cfg.cfg {
                predicates.push(cfg);
            }
            return Ok(Some(Variant {
                function: function.clone(),
                predicate: combine_cfg_predicates(&predicates),
                invocation: combine_cfg_predicates(&invocation),
            }));
        }
    }
    Ok(None)
}

fn scopes<'a>(items: &'a [Item], input: &ItemFn, candidates: &mut Vec<&'a [Item]>) {
    if items
        .iter()
        .any(|item| matches!(item, Item::Fn(function) if function.sig.ident == input.sig.ident))
    {
        candidates.push(items);
    }
    for item in items {
        if let Item::Mod(module) = item
            && let Some((_, items)) = &module.content
        {
            scopes(items, input, candidates);
        }
    }
}

pub fn collect(input: &ItemFn, cfg: &ExportCfg) -> syn::Result<Group> {
    let fallback = || {
        let mut predicates = extract_cfg_predicates(&input.attrs);
        predicates.extend(cfg.cfg.iter().cloned());
        Group {
            variants: vec![Variant {
                function: input.clone(),
                predicate: combine_cfg_predicates(&predicates),
                invocation: None,
            }],
            emitter: None,
        }
    };
    if !proc_macro::is_available() {
        return Ok(fallback());
    }
    let span = input.sig.ident.span();
    let Some(path) = span.unwrap().local_file() else {
        return Ok(fallback());
    };
    let Ok(source) = std::fs::read_to_string(path) else {
        return Ok(fallback());
    };
    let Ok(file) = syn::parse_file(&source) else {
        return Ok(fallback());
    };
    let mut candidates = Vec::new();
    scopes(&file.items, input, &mut candidates);
    // A definition-site position distinguishes identical signatures in
    // unrelated inline modules. Parsed-source spans use the same coordinates.
    let position = span.start();
    let scope = candidates
        .iter()
        .copied()
        .find(|items| {
            items.iter().any(|item| {
        matches!(item, Item::Fn(function) if function.sig.ident.span().start() == position)
    })
        })
        .or_else(|| (candidates.len() == 1).then(|| candidates[0]));
    let Some(scope) = scope else {
        return Ok(fallback());
    };
    let mut variants = Vec::new();
    for item in scope {
        if let Item::Fn(function) = item
            && function.sig.ident == input.sig.ident
            && let Some(variant) = source_variant(function)?
        {
            variants.push(variant);
        }
    }
    if variants.is_empty() {
        return Ok(fallback());
    }
    let current = variants
        .iter()
        .position(|v| v.function.sig.ident.span().start() == position)
        .or_else(|| {
            variants
                .iter()
                .position(|v| v.function.sig == input.sig && v.function.block == input.block)
        });
    let Some(current) = current else {
        return Ok(fallback());
    };
    variants[current].function = input.clone();
    let availability = variants[current]
        .invocation
        .clone()
        .unwrap_or_else(|| quote!(all()));
    let earlier = variants[..current]
        .iter()
        .map(|v| v.invocation.clone().unwrap_or_else(|| quote!(all())));
    let emitter = Some(quote!(all(#availability, not(any(#(#earlier),*)))));
    Ok(Group { variants, emitter })
}

pub fn attrs(variant: &Variant) -> Vec<syn::Attribute> {
    let mut attrs = variant.function.attrs.clone();
    attrs.retain(|attr| !attr.path().is_ident("cfg"));
    if let Some(predicate) = &variant.predicate {
        attrs.push(syn::parse_quote!(#[cfg(#predicate)]));
    }
    attrs
}
