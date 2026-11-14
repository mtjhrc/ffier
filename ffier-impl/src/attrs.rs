//! Parsing and inspection of shared export attributes.

use quote::ToTokens;

/// Extract `#[ffier(...)]` from an attribute — handles both plain
/// `#[ffier(...)]` and `#[cfg_attr(COND, ffier(...))]`.
/// Returns `None` for non-ffier attrs.
pub(crate) fn extract_ffier_attr(attr: &syn::Attribute) -> Option<syn::Attribute> {
    if attr.path().is_ident("ffier") {
        return Some(attr.clone());
    }
    if !attr.path().is_ident("cfg_attr") {
        return None;
    }
    let tokens = match &attr.meta {
        syn::Meta::List(list) => list.tokens.clone(),
        _ => return None,
    };

    // Find the top-level comma separating condition from the inner attr,
    // then check if the remainder starts with `ffier`.
    // Group tokens (parenthesized, braced, bracketed) are atomic in
    // proc_macro2 — a comma inside `all(feature = "ffi")` is inside the
    // group's stream and never appears as a top-level Punct.
    let mut after_comma = proc_macro2::TokenStream::new();
    let mut found_comma = false;

    for tt in tokens {
        if !found_comma {
            if let proc_macro2::TokenTree::Punct(p) = &tt
                && p.as_char() == ','
            {
                found_comma = true;
                continue;
            }
        } else {
            after_comma.extend(std::iter::once(tt));
        }
    }

    if !found_comma {
        return None;
    }

    let mut after_iter = after_comma.clone().into_iter();
    if let Some(proc_macro2::TokenTree::Ident(id)) = after_iter.next()
        && id == "ffier"
    {
        return Some(syn::parse_quote! { #[#after_comma] });
    }

    None
}

/// Strip all ffier attrs (direct or cfg_attr-wrapped) from an attribute list.
pub(crate) fn strip_ffier_attrs(attrs: &mut Vec<syn::Attribute>) {
    attrs.retain(|a| extract_ffier_attr(a).is_none());
}

/// True when the item derives `Copy` (directly or through a helper macro).
///
/// Value structs must be `Copy`: the by-value ABI treats them as plain data, so
/// drop glue would be unsound and unnecessary.
pub(crate) fn has_copy_derive(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("derive"))
        .any(|a| {
            let mut found = false;
            let _ = a.parse_nested_meta(|meta| {
                if meta.path.is_ident("Copy") {
                    found = true;
                }
                Ok(())
            });
            found
        })
}

/// Extract the `#[repr(X)]` attribute from an item, returning the ident X
/// if it's a supported integer repr.
pub(crate) fn extract_repr(attrs: &[syn::Attribute]) -> Option<syn::Ident> {
    for attr in attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        if let Ok(ident) = attr.parse_args::<syn::Ident>() {
            let s = ident.to_string();
            if matches!(
                s.as_str(),
                "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64"
            ) {
                return Some(ident);
            }
        }
    }
    None
}

/// Recognized `#[ffier(...)]` attributes on a parameter.
/// Supports dispatch selection and foreign handle metadata; rejects unknown keys.
pub(crate) struct FfierParamAttrs {
    pub(crate) dispatch: Option<String>,
    /// Absolute foreign ffier library crate path (e.g. `::other_lib`).
    pub(crate) foreign_crate: Option<syn::Path>,
    /// C typedef name for the foreign handle type (e.g. `"FlForeignConfig"`).
    pub(crate) foreign_c_name: Option<String>,
}

fn parse_foreign_crate_path(input: syn::parse::ParseStream) -> syn::Result<syn::Path> {
    let path: syn::Path = input.parse()?;
    if path.leading_colon.is_none() {
        return Err(syn::Error::new_spanned(
            path,
            "foreign crate path must start with `::`",
        ));
    }
    Ok(path)
}

pub(crate) fn parse_ffier_param_attrs(attrs: &[syn::Attribute]) -> syn::Result<FfierParamAttrs> {
    let mut result = FfierParamAttrs {
        dispatch: None,
        foreign_crate: None,
        foreign_c_name: None,
    };
    for raw_attr in attrs {
        let Some(attr) = extract_ffier_attr(raw_attr) else {
            continue;
        };
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("dispatch") {
                let value = meta.value()?;
                let mode: syn::Ident = value.parse()?;
                result.dispatch = Some(mode.to_string());
            } else if meta.path.is_ident("foreign") {
                result.foreign_crate = Some(parse_foreign_crate_path(meta.value()?)?);
            } else if meta.path.is_ident("c_name") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                result.foreign_c_name = Some(lit.value());
            } else {
                return Err(meta.error(format!(
                    "unknown #[ffier] key `{}` on parameter",
                    meta.path.to_token_stream(),
                )));
            }
            Ok(())
        })?;
    }
    Ok(result)
}

/// All recognized keys from `#[ffier(...)]` on a trait/impl method.
pub(crate) struct FfierMethodAttrs {
    pub(crate) index: Option<usize>,
    pub(crate) raw_handle: bool,
    pub(crate) dispatch: Option<String>,
    pub(crate) skip: bool,
    /// Absolute foreign ffier library crate path for the return type.
    pub(crate) foreign_return: Option<syn::Path>,
    /// C typedef name for the foreign return handle type.
    pub(crate) foreign_return_c_name: Option<String>,
}

/// Parse all `#[ffier(...)]` attributes on a method in one pass.
/// Rejects unknown keys.
pub(crate) fn parse_ffier_method_attrs(attrs: &[syn::Attribute]) -> syn::Result<FfierMethodAttrs> {
    let mut result = FfierMethodAttrs {
        index: None,
        raw_handle: false,
        dispatch: None,
        skip: false,
        foreign_return: None,
        foreign_return_c_name: None,
    };

    for raw_attr in attrs {
        let Some(attr) = extract_ffier_attr(raw_attr) else {
            continue;
        };
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("index") {
                let value = meta.value()?;
                let lit: syn::LitInt = value.parse()?;
                let index = lit.base10_parse::<usize>()?;
                // Slot count is index + 1, stored as u16 in the vtable header.
                if index >= usize::from(u16::MAX) {
                    return Err(meta.error(
                        "vtable index must leave room for the slot count to fit \
                         in u16, because the vtable header stores it as u16",
                    ));
                }
                result.index = Some(index);
            } else if meta.path.is_ident("raw_handle") {
                result.raw_handle = true;
            } else if meta.path.is_ident("dispatch") {
                let value = meta.value()?;
                let mode: syn::Ident = value.parse()?;
                result.dispatch = Some(mode.to_string());
            } else if meta.path.is_ident("skip") {
                result.skip = true;
            } else if meta.path.is_ident("foreign_return") {
                result.foreign_return = Some(parse_foreign_crate_path(meta.value()?)?);
            } else if meta.path.is_ident("c_name") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                result.foreign_return_c_name = Some(lit.value());
            } else {
                return Err(meta.error(format!(
                    "unknown #[ffier] key `{}`",
                    meta.path.to_token_stream(),
                )));
            }
            Ok(())
        })?;
    }

    Ok(result)
}

pub(crate) fn extract_doc_comments(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            let syn::Meta::NameValue(nv) = &attr.meta else {
                return None;
            };
            let syn::Expr::Lit(lit) = &nv.value else {
                return None;
            };
            let syn::Lit::Str(s) = &lit.lit else {
                return None;
            };
            Some(s.value())
        })
        .collect()
}
