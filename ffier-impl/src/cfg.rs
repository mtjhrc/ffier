use ffier_schema::Cfg;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Meta, Token, ext::IdentExt, parse::Parser, punctuated::Punctuated};

pub fn from_meta(meta: &Meta) -> syn::Result<Cfg> {
    let key = meta
        .path()
        .get_ident()
        .ok_or_else(|| syn::Error::new_spanned(meta, "expected cfg identifier"))?
        .unraw()
        .to_string();
    match meta {
        Meta::Path(_) => match key.as_str() {
            "windows" => Ok(Cfg::Windows),
            "unix" => Ok(Cfg::Unix),
            _ => Err(syn::Error::new_spanned(
                meta,
                "unsupported cfg flag; expected windows or unix",
            )),
        },
        Meta::NameValue(nv) => {
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(value),
                ..
            }) = &nv.value
            else {
                return Err(syn::Error::new_spanned(
                    &nv.value,
                    "expected cfg string value",
                ));
            };
            match key.as_str() {
                "target_os" => Ok(Cfg::TargetOs(value.value())),
                "feature" => Ok(Cfg::Feature(value.value())),
                _ => Err(syn::Error::new_spanned(
                    meta,
                    "unsupported cfg key; expected target_os or feature",
                )),
            }
        }
        Meta::List(list) => {
            let items = Punctuated::<Meta, Token![,]>::parse_terminated
                .parse2(list.tokens.clone())?
                .iter()
                .map(from_meta)
                .collect::<syn::Result<Vec<_>>>()?;
            match key.as_str() {
                "all" => Ok(Cfg::All(items)),
                "any" => Ok(Cfg::Any(items)),
                "not" if items.len() == 1 => {
                    Ok(Cfg::Not(Box::new(items.into_iter().next().unwrap())))
                }
                _ => Err(syn::Error::new_spanned(
                    meta,
                    "expected all(...), any(...), or not(predicate)",
                )),
            }
        }
    }
}

/// Parse one predicate, including the optional comma accepted by cfg attributes.
pub fn parse_meta(input: syn::parse::ParseStream) -> syn::Result<Meta> {
    let meta = input.parse()?;
    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    }
    Ok(meta)
}

pub fn from_tokens(tokens: &TokenStream) -> syn::Result<Cfg> {
    from_meta(&parse_meta.parse2(tokens.clone())?)
}

fn predicate_tokens(cfg: &Cfg) -> TokenStream {
    match cfg {
        Cfg::Windows => quote!(windows),
        Cfg::Unix => quote!(unix),
        Cfg::TargetOs(value) => quote!(target_os = #value),
        Cfg::Feature(value) => quote!(feature = #value),
        Cfg::All(items) | Cfg::Any(items) => {
            let items = items.iter().map(predicate_tokens);
            if matches!(cfg, Cfg::All(_)) {
                quote!(all(#(#items),*))
            } else {
                quote!(any(#(#items),*))
            }
        }
        Cfg::Not(item) => {
            let item = predicate_tokens(item);
            quote!(not(#item))
        }
    }
}

/// Canonical tokens can safely be composed inside other cfg predicates.
pub fn normalize_tokens(tokens: &TokenStream) -> syn::Result<TokenStream> {
    Ok(predicate_tokens(&from_tokens(tokens)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;
    use serde_json::json;

    #[test]
    fn parses_nested_cfg_tokens_into_objects() {
        let cfg = from_tokens(&quote!(all(
            unix,
            not(target_os = "macos"),
            any(feature = "files",)
        )))
        .unwrap();
        assert_eq!(
            serde_json::to_value(cfg).unwrap(),
            json!({"kind": "all", "value": [
                {"kind": "unix"},
                {"kind": "not", "value": {"kind": "target_os", "value": "macos"}},
                {"kind": "any", "value": [{"kind": "feature", "value": "files"}]}
            ]})
        );
    }

    #[test]
    fn accepts_and_normalizes_trailing_commas() {
        for tokens in [quote!(unix,), quote!(all(unix,),), quote!(not(windows,),)] {
            let expected = from_tokens(&tokens).unwrap();
            let normalized = normalize_tokens(&tokens).unwrap();
            assert!(!normalized.to_string().ends_with(','));
            assert_eq!(from_tokens(&normalized).unwrap(), expected);
        }
        let first = normalize_tokens(&quote!(unix,)).unwrap();
        let second = normalize_tokens(&quote!(feature = "files",)).unwrap();
        assert_eq!(
            from_tokens(&quote!(all(#first, #second))).unwrap(),
            Cfg::All(vec![Cfg::Unix, Cfg::Feature("files".into())])
        );
    }

    #[test]
    fn empty_combinators_represent_constants() {
        assert_eq!(from_tokens(&quote!(all())).unwrap(), Cfg::All(vec![]));
        assert_eq!(from_tokens(&quote!(any())).unwrap(), Cfg::Any(vec![]));
    }

    #[test]
    fn rejects_unsupported_and_invalid_cfg_tokens() {
        for tokens in [
            quote!(custom),
            quote!(target_arch = "x86_64"),
            quote!(target_family = "unix"),
            quote!(target_pointer_width = "64"),
            quote!(target_feature = "sse"),
            quote!(all(unix, custom)),
            quote!(feature),
            quote!(unix = "yes"),
            quote!(not()),
            quote!(not(unix, windows)),
            quote!(target_os = 42),
            quote!(other(unix)),
            quote!(unix,,),
            quote!(unix, windows),
        ] {
            assert!(from_tokens(&tokens).is_err(), "{tokens}");
        }
    }

    #[test]
    fn direct_tokens_preserve_escaped_values() {
        let original = quote!(feature = "files\"\\\n");
        assert_eq!(
            from_tokens(&normalize_tokens(&original).unwrap()).unwrap(),
            from_tokens(&original).unwrap()
        );
    }

    #[test]
    fn metadata_capture_accepts_root_trailing_comma() {
        let attrs: Vec<syn::Attribute> = vec![syn::parse_quote!(#[cfg(unix,)])];
        let predicates = crate::extract_cfg_predicates(&attrs).unwrap();
        assert_eq!(predicates.len(), 1);
        assert_eq!(
            from_tokens(&crate::combine_cfg_predicates(&predicates).unwrap()).unwrap(),
            Cfg::Unix
        );
        assert_eq!(
            from_tokens(
                &parse_meta
                    .parse2(quote!(feature = "files",))
                    .unwrap()
                    .to_token_stream()
            )
            .unwrap(),
            Cfg::Feature("files".into())
        );
    }
}
