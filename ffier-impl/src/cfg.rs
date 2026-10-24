use ffier_schema::Cfg;
use syn::{Meta, Token, parse::Parser, punctuated::Punctuated};

pub fn from_meta(meta: &Meta) -> syn::Result<Cfg> {
    let key = meta
        .path()
        .get_ident()
        .ok_or_else(|| syn::Error::new_spanned(meta, "expected cfg identifier"))?
        .to_string();
    match meta {
        Meta::Path(_) => Ok(Cfg::Flag(key)),
        Meta::NameValue(nv) => {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(value),
                ..
            }) = &nv.value
            {
                Ok(Cfg::Value {
                    key,
                    value: value.value(),
                })
            } else {
                Err(syn::Error::new_spanned(
                    &nv.value,
                    "expected cfg string value",
                ))
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

pub fn from_tokens(tokens: &proc_macro2::TokenStream) -> syn::Result<Cfg> {
    from_meta(&syn::parse2(tokens.clone())?)
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
