//! Collect sibling free-function signatures before Rust's item-level cfg
//! filtering loses the inactive alternatives. Metadata still uses the same
//! cfg predicates and method parser as exported impls.
//!
//! This is derived from the definition's source, not an expansion-order
//! registry: separate proc-macro processes and incremental builds produce the
//! same metadata. Macro-generated functions retain the single-item path.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::{cell::RefCell, collections::HashMap, path::PathBuf};
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
    let mut exports = Vec::new();
    for attr in &function.attrs {
        if let Some(export) = export_args(&attr.meta, &[])? {
            exports.push(export);
        }
    }
    if exports.is_empty() {
        return Ok(None);
    }
    let direct = extract_cfg_predicates(&function.attrs);
    let first_cfg = exports[0].0.cfg.as_ref().map(ToString::to_string);
    let shared = exports
        .iter()
        .all(|(cfg, _)| cfg.cfg.as_ref().map(ToString::to_string) == first_cfg);
    let mut predicates = direct.clone();
    if shared {
        predicates.extend(exports[0].0.cfg.iter().cloned());
    } else {
        let choices = exports.iter().map(|(cfg, conditions)| {
            let cfg = cfg.cfg.clone().unwrap_or_else(|| quote!(all()));
            quote!(all(#(#conditions,)* #cfg))
        });
        predicates.push(quote!(any(#(#choices),*)));
    }
    let invocations: Vec<_> = exports
        .iter()
        .map(|(_, conditions)| quote!(all(#(#conditions,)* #(#direct),*)))
        .collect();
    let invocation = if invocations.len() == 1 {
        invocations[0].clone()
    } else {
        quote!(any(#(#invocations),*))
    };
    Ok(Some(Variant {
        function: function.clone(),
        predicate: combine_cfg_predicates(&predicates),
        invocation: Some(invocation),
    }))
}

#[derive(Clone)]
struct SourceFunction {
    position: proc_macro2::LineColumn,
    tokens: String,
}

struct SourceFile {
    text: String,
    scopes: Vec<Vec<SourceFunction>>,
}

thread_local! {
    static SOURCES: RefCell<HashMap<PathBuf, SourceFile>> = RefCell::new(HashMap::new());
}

fn collect_scopes(items: &[Item], output: &mut Vec<Vec<SourceFunction>>) {
    let functions: Vec<_> = items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) => Some(SourceFunction {
                position: function.sig.ident.span().start(),
                tokens: function.to_token_stream().to_string(),
            }),
            _ => None,
        })
        .collect();
    if !functions.is_empty() {
        output.push(functions);
    }
    for item in items {
        if let Item::Mod(module) = item
            && let Some((_, items)) = &module.content
        {
            collect_scopes(items, output);
        }
    }
}

fn source_scope(
    path: PathBuf,
    text: String,
    position: proc_macro2::LineColumn,
) -> Option<Vec<SourceFunction>> {
    SOURCES.with(|sources| {
        let mut sources = sources.borrow_mut();
        if sources.get(&path).is_none_or(|file| file.text != text) {
            // Compiler parsing assigns the invocation's span to all tokens.
            // The positioned fallback lexer retains real source coordinates.
            // All macro entry points hold macro_context() during this switch.
            struct PositionedLexer;
            impl Drop for PositionedLexer {
                fn drop(&mut self) {
                    proc_macro2::fallback::unforce();
                }
            }
            proc_macro2::fallback::force();
            let _lexer = PositionedLexer;
            let file = syn::parse_file(&text).ok()?;
            let mut scopes = Vec::new();
            collect_scopes(&file.items, &mut scopes);
            sources.insert(path.clone(), SourceFile { text, scopes });
        }
        sources
            .get(&path)?
            .scopes
            .iter()
            .find(|scope| scope.iter().any(|function| function.position == position))
            .cloned()
    })
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
    let Ok(source) = std::fs::read_to_string(&path) else {
        return Ok(fallback());
    };
    let position = span.start();
    let Some(scope) = source_scope(path, source, position) else {
        return Ok(fallback());
    };
    let mut variants = Vec::new();
    let mut current = None;
    for source in scope {
        let function: ItemFn = syn::parse_str(&source.tokens)?;
        if function.sig.ident == input.sig.ident
            && let Some(variant) = source_variant(&function)?
        {
            if source.position == position {
                current = Some(variants.len());
            }
            variants.push(variant);
        }
    }
    if variants.is_empty() {
        return Ok(fallback());
    }
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
