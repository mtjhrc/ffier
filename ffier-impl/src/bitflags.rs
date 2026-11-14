//! Definition and FFI metadata generation for bitflags.

use std::sync::atomic::Ordering;

use ffier_schema::ThreadSafety;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Token, parse::Parse};

use crate::MACRO_COUNTER;
use crate::cfg::{self, combine_cfg_predicates};
use crate::export::ExportCfg;
use crate::meta::camel_to_snake;
use crate::reexport::reexport_helpers;

pub(crate) fn export_bitflags(input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<ExportBitflagsInput>(input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };

    let bitflags_call = &parsed.bitflags_call;
    let name = &parsed.name;
    let repr_ident = &parsed.repr;
    let repr_str = repr_ident.to_string();
    let export_cfg = ExportCfg {
        cfg: combine_cfg_predicates(&parsed.cfg_predicates),
        thread_safety: ThreadSafety::default(),
        lifetime_variance: Vec::new(),
    };

    let mut variants_meta = Vec::new();
    for (flag_name, value) in &parsed.flags {
        variants_meta.push(quote! {
            { name = #flag_name, value = #value, }
        });
    }

    let bf_snake = camel_to_snake(&name.to_string());
    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{bf_snake}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{name}");
    let helper_mod_name = format_ident!("_ffier_{bf_snake}");

    let cfg_pred_tokens = export_cfg.meta_tokens();
    let cfg_attr = export_cfg.cfg_attr();

    let (reexport_helpers, reexport_bundle) = reexport_helpers(&[&helper_mod_name], &cfg_attr);

    let output = quote! {
        #reexport_bundle

        #cfg_attr
        #bitflags_call

        // macro_rules! cannot be gated with #[cfg] directly and
        // #[macro_export] cannot appear inside const blocks, so the
        // macro is always emitted but the pub-use alias that makes it
        // reachable is gated behind the feature flag.
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            #reexport_helpers
            (@on_library_export, $__type_tag:expr, [$($__handle:ident),*], [$($__value:ident),*]) => {
                impl FfiType for #name {
                    type CRepr = #repr_ident;
                    const C_TYPE_NAME: &'static str = stringify!(#name);
                    const IS_HANDLE: bool = false;
                    fn into_c(self) -> #repr_ident { self.bits() }
                    unsafe fn from_c(repr: #repr_ident) -> Self { Self::from_bits_retain(repr) }
                }
                impl FfiBorrow for #name {
                    fn borrow_as_c(&self) -> #repr_ident { self.bits() }
                }
            };
            // Tagged invocation (from library_definition! shim): includes prefix
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exported_bitflags,
                    name = #name,
                    prefix = $prefix,
                    repr = #repr_str,
                    variants = [#(#variants_meta),*],
                    #cfg_pred_tokens
                } $(, $($rest)*)? }
            };
        }

        #cfg_attr
        #[doc(hidden)]
        pub mod #helper_mod_name {}

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
    };

    output.into()
}

/// Parsed input for `export_bitflags!`:
/// ```ignore
/// bitflags::bitflags! {
///     [attrs]
///     pub struct Name: repr { const FLAG = val; ... }
/// }
/// ```
///
/// An optional `#[cfg(...)]` attribute may precede the `bitflags!` call to
/// gate the type behind a Cargo feature while keeping the FFI metadata
/// unconditionally available:
/// ```ignore
/// ffier::export_bitflags! {
///     #[cfg(feature = "optional")]
///     bitflags::bitflags! { ... }
/// }
/// ```
struct ExportBitflagsInput {
    /// `#[cfg(...)]` predicates extracted from before the `bitflags!` call.
    cfg_predicates: Vec<syn::Meta>,
    /// The complete `bitflags! { ... }` or `bitflags::bitflags! { ... }`
    /// invocation, preserved verbatim for re-emission.
    bitflags_call: proc_macro2::TokenStream,
    /// Struct name extracted from the inner body.
    name: syn::Ident,
    /// Repr type (u8/u16/u32/u64/i8/i16/i32/i64).
    repr: syn::Ident,
    /// Flag constants: (name, value).
    flags: Vec<(syn::Ident, u64)>,
}

impl Parse for ExportBitflagsInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // Parse optional leading #[cfg(...)] attributes.
        let mut cfg_predicates = Vec::new();
        while input.peek(Token![#]) && input.peek2(syn::token::Bracket) {
            let attrs = input.call(syn::Attribute::parse_outer)?;
            for attr in attrs {
                if attr.path().is_ident("cfg") {
                    let predicate = attr.parse_args_with(cfg::parse_meta)?;
                    cfg::from_meta(&predicate)?;
                    cfg_predicates.push(predicate);
                } else {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "export_bitflags! only supports #[cfg(...)] attributes before the bitflags! call",
                    ));
                }
            }
        }

        // Capture the remaining tokens as the bitflags! call.
        // We need to parse them to extract name/repr/flags, but also
        // preserve the original tokens for verbatim re-emission.
        let remaining: proc_macro2::TokenStream = input.parse()?;
        let bitflags_call = remaining.clone();

        // Parse the bitflags macro call to extract struct info.
        // Expected forms:
        //   bitflags! { ... }
        //   bitflags::bitflags! { ... }
        //   path::to::bitflags! { ... }
        let body = parse_bitflags_body(remaining)?;

        Ok(ExportBitflagsInput {
            cfg_predicates,
            bitflags_call,
            name: body.name,
            repr: body.repr,
            flags: body.flags,
        })
    }
}

/// Extract struct name, repr, and flag constants from a `bitflags! { ... }` call.
///
/// Parses through the macro path to find the braced body, then scans for
/// `struct Name: repr { const FLAG = val; ... }` inside it, skipping any
/// leading attributes.
fn parse_bitflags_body(tokens: proc_macro2::TokenStream) -> syn::Result<BitflagsStructBody> {
    // Find the `!` and the braced body after the macro path.
    let mut iter = tokens.into_iter().peekable();
    let mut found_bang = false;

    for tt in iter.by_ref() {
        if let proc_macro2::TokenTree::Punct(p) = &tt
            && p.as_char() == '!'
        {
            found_bang = true;
            break;
        }
    }

    if !found_bang {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "expected `bitflags! { ... }` macro invocation",
        ));
    }

    // The next token should be a brace-delimited group
    let body_group = match iter.next() {
        Some(proc_macro2::TokenTree::Group(g))
            if g.delimiter() == proc_macro2::Delimiter::Brace =>
        {
            g
        }
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "expected braced body after `bitflags!`",
            ));
        }
    };

    // Parse the inner body to find `struct Name: repr { ... }`
    // Skip leading attributes (#[...]) and visibility (pub)
    let body_stream = body_group.stream();
    syn::parse2::<BitflagsStructBody>(body_stream)
}

/// Parser for the inside of a `bitflags! { ... }` body.
/// Skips attributes, parses `[pub] struct Name: repr { const FLAG = val; ... }`.
struct BitflagsStructBody {
    name: syn::Ident,
    repr: syn::Ident,
    flags: Vec<(syn::Ident, u64)>,
}

impl Parse for BitflagsStructBody {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // Skip outer attributes (#[derive(...)], #[repr(...)], etc.)
        let _ = input.call(syn::Attribute::parse_outer)?;

        // Skip optional visibility
        let _: Option<syn::Visibility> = if input.peek(Token![pub]) {
            Some(input.parse()?)
        } else {
            None
        };

        // `struct`
        input.parse::<Token![struct]>()?;

        // Name
        let name: syn::Ident = input.parse()?;

        // `: repr`
        input.parse::<Token![:]>()?;
        let repr: syn::Ident = input.parse()?;

        // Validate repr
        let repr_s = repr.to_string();
        if !matches!(
            repr_s.as_str(),
            "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64"
        ) {
            return Err(syn::Error::new(
                repr.span(),
                "bitflags repr must be one of u8, u16, u32, u64, i8, i16, i32, i64",
            ));
        }

        // `{ const FLAG = val; ... }`
        let content;
        syn::braced!(content in input);

        let mut flags = Vec::new();
        while !content.is_empty() {
            // Skip attributes on individual flags (e.g. #[doc = "..."])
            let _ = content.call(syn::Attribute::parse_outer)?;

            if content.is_empty() {
                break;
            }

            content.parse::<Token![const]>()?;
            let flag_name: syn::Ident = content.parse()?;
            content.parse::<Token![=]>()?;

            let lit: syn::LitInt = content.parse()?;
            let value = lit.base10_parse::<u64>()?;

            flags.push((flag_name, value));

            // Optional semicolon and/or comma
            let _ = content.parse::<Token![;]>();
            let _ = content.parse::<Token![,]>();
        }

        Ok(BitflagsStructBody { name, repr, flags })
    }
}
