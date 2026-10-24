use ffier_schema::Cfg;
use syn::{
    Token,
    ext::IdentExt,
    parse::{Parse, Parser},
    punctuated::Punctuated,
};

struct Predicate(Cfg);

impl Parse for Predicate {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.peek(syn::LitBool) {
            let value: syn::LitBool = input.parse()?;
            return Ok(Self(if value.value {
                Cfg::All(vec![])
            } else {
                Cfg::Any(vec![])
            }));
        }
        let key = input.call(syn::Ident::parse_any)?.unraw().to_string();
        let cfg = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            let value: syn::LitStr = input.parse()?;
            Cfg::Value {
                key,
                value: value.value(),
            }
        } else if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            let items = Punctuated::<Predicate, Token![,]>::parse_terminated(&content)?
                .into_iter()
                .map(|item| item.0)
                .collect::<Vec<_>>();
            match key.as_str() {
                "all" => Cfg::All(items),
                "any" => Cfg::Any(items),
                "not" if items.len() == 1 => Cfg::Not(Box::new(items.into_iter().next().unwrap())),
                _ => return Err(input.error("expected all(...), any(...), or not(predicate)")),
            }
        } else {
            Cfg::Flag(key)
        };
        Ok(Self(cfg))
    }
}

pub fn from_tokens(tokens: &proc_macro2::TokenStream) -> syn::Result<Cfg> {
    let parser = |input: syn::parse::ParseStream| {
        let predicate: Predicate = input.parse()?;
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        Ok(predicate.0)
    };
    parser.parse2(tokens.clone())
}

/// Canonical tokens can safely be composed inside other cfg predicates.
pub fn normalize_tokens(
    tokens: &proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    Ok(from_tokens(tokens)?
        .to_string()
        .parse()
        .expect("valid cfg tokens"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
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
            json!({"all": [
                {"unix": true}, {"not": {"target_os": "macos"}}, {"any": [{"feature": "files"}]}
            ]})
        );
    }

    #[test]
    fn boolean_predicates_use_empty_combinators() {
        assert_eq!(from_tokens(&quote!(true)).unwrap(), Cfg::All(vec![]));
        assert_eq!(from_tokens(&quote!(false)).unwrap(), Cfg::Any(vec![]));
        assert_eq!(
            from_tokens(&quote!(all(unix, not(false), true))).unwrap(),
            Cfg::All(vec![
                Cfg::Flag("unix".into()),
                Cfg::Not(Box::new(Cfg::Any(vec![]))),
                Cfg::All(vec![])
            ])
        );
    }

    #[test]
    fn accepts_and_normalizes_a_trailing_comma() {
        for tokens in [quote!(unix,), quote!(all(unix,),)] {
            assert!(from_tokens(&tokens).is_ok());
            assert!(
                !normalize_tokens(&tokens)
                    .unwrap()
                    .to_string()
                    .ends_with(',')
            );
        }
        assert_eq!(from_tokens(&quote!(true,)).unwrap(), Cfg::All(vec![]));
        assert!(from_tokens(&quote!(unix,,)).is_err());
        assert!(from_tokens(&quote!(unix, windows)).is_err());
    }

    #[test]
    fn rejects_invalid_cfg_tokens() {
        for tokens in [
            quote!(not()),
            quote!(not(unix, windows)),
            quote!(target_os = 42),
            quote!(other(unix)),
        ] {
            assert!(from_tokens(&tokens).is_err());
        }
    }
}
