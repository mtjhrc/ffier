//! Export integer-representation enums.

use std::sync::atomic::Ordering;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput};

use super::ExportCfg;
use crate::{MACRO_COUNTER, attrs::extract_repr, meta::camel_to_snake, reexport::reexport_helpers};

/// Handle `#[ffier::export]` on a `#[repr(uN)]` enum.
///
/// Extracts the repr type and variant discriminants, emits a metadata macro
/// that bridges to the schema generator. Also generates a `FfiType` impl
/// so the enum can be used as a parameter/return type in exported methods.
pub(super) fn exportable_enum(input: DeriveInput, export_cfg: &ExportCfg) -> TokenStream {
    let name = &input.ident;
    let Data::Enum(data_enum) = &input.data else {
        unreachable!();
    };

    // Extract #[repr(uN)] — required for enum constants.
    let repr_ident = match extract_repr(&input.attrs) {
        Some(r) => r,
        None => {
            return syn::Error::new_spanned(
                &input,
                "#[ffier::export] on enums requires #[repr(u8/u16/u32/u64/i8/i16/i32/i64)]",
            )
            .to_compile_error()
            .into();
        }
    };
    let repr_str = repr_ident.to_string();

    // Collect variant names and discriminant values.
    let mut variants_meta = Vec::new();
    let mut variant_values: Vec<(syn::Ident, u64)> = Vec::new();
    let mut next_value: u64 = 0;
    for variant in &data_enum.variants {
        if !variant.fields.is_empty() {
            return syn::Error::new_spanned(
                variant,
                "#[ffier::export] enums must have unit variants only (no fields)",
            )
            .to_compile_error()
            .into();
        }
        let value = if let Some((_, expr)) = &variant.discriminant {
            // Parse the discriminant expression as a literal integer.
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(lit),
                ..
            }) = expr
            {
                match lit.base10_parse::<u64>() {
                    Ok(v) => v,
                    Err(e) => return e.to_compile_error().into(),
                }
            } else {
                return syn::Error::new_spanned(
                    expr,
                    "ffier: enum discriminant must be a literal integer",
                )
                .to_compile_error()
                .into();
            }
        } else {
            next_value
        };
        next_value = value + 1;
        let var_ident = &variant.ident;
        variant_values.push((var_ident.clone(), value));
        variants_meta.push(quote! {
            { name = #var_ident, value = #value, }
        });
    }

    // Build match arms for from_c: each discriminant maps to its variant,
    // unknown values panic.
    let from_c_arms: Vec<_> = variant_values
        .iter()
        .map(|(ident, val)| {
            let lit = proc_macro2::Literal::u64_unsuffixed(*val);
            quote! { #lit => Self::#ident, }
        })
        .collect();
    let name_str = name.to_string();

    let enum_snake = camel_to_snake(&name.to_string());
    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{enum_snake}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{name}");
    let helper_mod_name = format_ident!("_ffier_{enum_snake}");

    let cfg_pred_tokens = export_cfg.meta_tokens();
    let cfg_attr = export_cfg.cfg_attr();

    let (reexport_helpers, reexport_bundle) = reexport_helpers(&[&helper_mod_name], &cfg_attr);

    let output = quote! {
        #reexport_bundle

        #cfg_attr
        #input

        #cfg_attr
        #[doc(hidden)]
        pub mod #helper_mod_name {}

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            #reexport_helpers
            (@on_library_export, $__type_tag:expr, [$($__handle:ident),*], [$($__value:ident),*]) => {
                impl FfiType for #name {
                    type CRepr = #repr_ident;
                    const C_TYPE_NAME: &'static str = stringify!(#name);
                    const IS_HANDLE: bool = false;
                    fn into_c(self) -> #repr_ident { self as #repr_ident }
                    unsafe fn from_c(repr: #repr_ident) -> Self {
                        match repr {
                            #(#from_c_arms)*
                            unknown => panic!(
                                "invalid {} discriminant: {}",
                                #name_str, unknown
                            ),
                        }
                    }
                }
                impl FfiBorrow for #name {
                    fn borrow_as_c(&self) -> #repr_ident { *self as #repr_ident }
                }
            };
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exported_enum,
                    name = #name,
                    prefix = $prefix,
                    repr = #repr_str,
                    variants = [#(#variants_meta),*],
                    #cfg_pred_tokens
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
    };

    output.into()
}
