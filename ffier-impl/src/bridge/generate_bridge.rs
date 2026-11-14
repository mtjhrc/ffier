//! Parse `generate_bridge!` arguments and invoke the library's metadata macro.

use proc_macro::TokenStream;
use quote::quote;

pub(crate) fn generate_bridge(input: TokenStream) -> TokenStream {
    let input2: proc_macro2::TokenStream = input.into();
    let mut iter = input2.into_iter().peekable();

    // Parse mode keyword
    let mode_ident = match iter.next() {
        Some(proc_macro2::TokenTree::Ident(id)) => id,
        _ => {
            return quote! {
                compile_error!("expected `local = <path>, schema_output = \"...\"` or `external = <path>, schema_output = \"...\"`");
            }
            .into();
        }
    };

    let mode_str = mode_ident.to_string();
    let mode = match mode_str.as_str() {
        "local" => quote! { @local },
        "external" => quote! { @cross_crate },
        _ => {
            return quote! {
                compile_error!(concat!(
                    "unknown mode `", stringify!(#mode_ident),
                    "`: expected `local` or `external`"
                ));
            }
            .into();
        }
    };

    // Consume `=`
    match iter.next() {
        Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '=' => {}
        _ => {
            return quote! {
                compile_error!("expected `=` after mode keyword");
            }
            .into();
        }
    }

    // Collect metadata macro path tokens until `,`
    let mut path_tokens = Vec::new();
    loop {
        match iter.peek() {
            Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == ',' => {
                iter.next(); // consume `,`
                break;
            }
            Some(_) => path_tokens.push(iter.next().unwrap()),
            None => {
                return quote! {
                    compile_error!("expected `, schema_output = \"...\"` after metadata macro path");
                }
                .into();
            }
        }
    }
    let path: proc_macro2::TokenStream = path_tokens.into_iter().collect();
    if path.is_empty() {
        return quote! {
            compile_error!("expected metadata macro path after `=`");
        }
        .into();
    }

    // Parse `schema_output = "..."`
    match iter.next() {
        Some(proc_macro2::TokenTree::Ident(id)) if id == "schema_output" => {}
        _ => {
            return quote! {
                compile_error!("expected `schema_output = \"...\"`");
            }
            .into();
        }
    }
    match iter.next() {
        Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '=' => {}
        _ => {
            return quote! {
                compile_error!("expected `=` after `schema_output`");
            }
            .into();
        }
    }
    let schema_output = match iter.next() {
        Some(proc_macro2::TokenTree::Literal(lit)) => lit,
        _ => {
            return quote! {
                compile_error!("expected string literal for schema_output path");
            }
            .into();
        }
    };

    if iter.next().is_some() {
        return quote! {
            compile_error!("unexpected trailing tokens after schema_output");
        }
        .into();
    }

    quote! { #path!(#mode, #schema_output); }.into()
}
