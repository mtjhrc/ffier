//! `FfiError` derive expansion and error attribute parsing.

use std::sync::atomic::Ordering;

use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Data, DeriveInput, LitStr, parse_macro_input};

use crate::MACRO_COUNTER;
use crate::attrs::extract_ffier_attr;
use crate::meta::{MetaErrorField, MetaErrorVariantData, camel_to_snake, camel_to_upper_snake};
use crate::reexport::reexport_helpers;

pub(crate) fn derive_ffi_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let Data::Enum(data_enum) = &input.data else {
        return syn::Error::new_spanned(&input, "FfiError can only be derived for enums")
            .to_compile_error()
            .into();
    };

    // Validate: enum must be #[non_exhaustive]
    if !input
        .attrs
        .iter()
        .any(|a| a.path().is_ident("non_exhaustive"))
    {
        return syn::Error::new_spanned(&input, "FfiError enums must be #[non_exhaustive]")
            .to_compile_error()
            .into();
    }

    for variant in &data_enum.variants {
        if matches!(variant.fields, syn::Fields::Named(_)) {
            return syn::Error::new_spanned(
                variant,
                "FfiError: named fields in variants are not yet supported; \
                 use tuple variants like `Variant(Box<str>)`",
            )
            .to_compile_error()
            .into();
        }
    }

    let mut code_arms = Vec::new();
    let mut message_arms = Vec::new();
    let mut codes_entries = Vec::new();
    let mut variant_meta_tokens = Vec::new();
    let mut payload_arms = Vec::new();
    let mut from_code_arms = Vec::new();

    for variant in &data_enum.variants {
        let var_ident = &variant.ident;

        let attrs = match parse_ffier_variant_attrs(&variant.attrs) {
            Ok(a) => a,
            Err(e) => return e.to_compile_error().into(),
        };

        let code = attrs.code;
        match &variant.fields {
            syn::Fields::Unit => from_code_arms.push(quote! { #code => Some(Self::#var_ident) }),
            syn::Fields::Unnamed(fields) if fields.unnamed.is_empty() => {
                from_code_arms.push(quote! { #code => Some(Self::#var_ident()) });
            }
            _ => {}
        }
        let message = attrs.message.unwrap_or_else(|| var_ident.to_string());
        let upper_name = camel_to_upper_snake(&var_ident.to_string());

        // Wildcard pattern for variants with fields
        let match_pattern = match &variant.fields {
            syn::Fields::Unit => quote! { #name::#var_ident },
            syn::Fields::Unnamed(_) => quote! { #name::#var_ident(..) },
            syn::Fields::Named(_) => quote! { #name::#var_ident { .. } },
        };
        code_arms.push(quote! { #match_pattern => #code });

        let msg_with_nul = format!("{message}\0");
        let msg_lit = proc_macro2::Literal::byte_string(msg_with_nul.as_bytes());
        message_arms.push(quote! {
            #code => unsafe {
                core::ffi::CStr::from_bytes_with_nul_unchecked(#msg_lit)
            }
        });

        let data = match &variant.fields {
            syn::Fields::Unit => MetaErrorVariantData::Fieldless,
            syn::Fields::Unnamed(fields) => {
                let fields = match fields
                    .unnamed
                    .iter()
                    .map(|field| parse_ffier_error_field(field, attrs.opaque))
                    .collect::<syn::Result<Vec<_>>>()
                {
                    Ok(fields) => fields,
                    Err(error) => return error.to_compile_error().into(),
                };
                MetaErrorVariantData::Tuple(fields)
            }
            syn::Fields::Named(_) => unreachable!("named fields rejected above"),
        };

        if let MetaErrorVariantData::Tuple(fields) = &data {
            let field_count = fields.len();
            let mut bindings = Vec::new();
            let mut field_arms = Vec::new();
            for (index, field) in fields.iter().enumerate() {
                match field {
                    MetaErrorField::Opaque => {
                        bindings.push(quote! { _ });
                        field_arms.push(quote! {
                            #index => panic!("{}::{} field {} is opaque", stringify!(#name), stringify!(#var_ident), index)
                        });
                    }
                    MetaErrorField::Typed(field_ty) => {
                        let binding = format_ident!("__field_{index}");
                        bindings.push(quote! { #binding });
                        field_arms.push(quote! {
                            #index => {
                                let expected = core::mem::size_of::<<#field_ty as FfiType>::CRepr>();
                                assert!(
                                    buf_size >= expected,
                                    "error payload buffer too small: got {} bytes, need {}",
                                    buf_size, expected,
                                );
                                let c_val = <#field_ty as FfiBorrow>::borrow_as_c(#binding);
                                unsafe {
                                    core::ptr::write(
                                        out_buf as *mut <#field_ty as FfiType>::CRepr,
                                        c_val,
                                    );
                                }
                            }
                        });
                    }
                }
            }
            payload_arms.push(quote! {
                #name::#var_ident(#(#bindings),*) => match index {
                    #(#field_arms,)*
                    _ => panic!("{}::{} field index {} out of bounds for {} fields", stringify!(#name), stringify!(#var_ident), index, #field_count),
                }
            });
        } else {
            payload_arms.push(quote! {
                #name::#var_ident => panic!("{}::{} field index {} out of bounds for 0 fields", stringify!(#name), stringify!(#var_ident), index),
            });
        }

        codes_entries.push(quote! { (#upper_name, #code) });
        variant_meta_tokens.push(quote! {
            { name = #var_ident, code = #code, message = #message, data = #data, }
        });
    }

    let unknown_msg = format!(
        "unknown {} error\0",
        camel_to_snake(&name.to_string()).replace('_', " ")
    );
    let unknown_lit = proc_macro2::Literal::byte_string(unknown_msg.as_bytes());

    let error_snake = camel_to_snake(&name.to_string());
    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{error_snake}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{name}");

    let error_path = quote! { $crate::#name };
    let helper_mod_name = format_ident!("_ffier_{error_snake}");

    let error_impl_helper = format_ident!("_ffier_impl_error_for_{error_snake}");
    let error_impl_meta = format_ident!("__ffier_meta_Error_for_{name}");
    let (reexport_helpers, reexport_bundle) = reexport_helpers(
        &[&helper_mod_name, &error_impl_helper, &error_impl_meta],
        &quote! {},
    );

    let output = quote! {
        #reexport_bundle

        #[doc(hidden)]
        pub mod #helper_mod_name {}

        impl ffier::FfiError for #name {
            fn code(&self) -> u32 {
                match self {
                    #(#code_arms,)*
                }
            }

            fn static_message(code: u32) -> &'static core::ffi::CStr {
                match code {
                    #(#message_arms,)*
                    _ => unsafe {
                        core::ffi::CStr::from_bytes_with_nul_unchecked(#unknown_lit)
                    },
                }
            }

            fn codes() -> &'static [(&'static str, u32)] {
                &[#(#codes_entries),*]
            }

            fn from_code(code: u32) -> Option<Self> {
                match code {
                    #(#from_code_arms,)*
                    _ => None,
                }
            }

            unsafe fn payload(&self, index: usize, out_buf: *mut core::ffi::c_void, buf_size: usize) {
                match self {
                    #(#payload_arms)*
                }
            }
        }

        #[ffier::export]
        impl ffier::Error for #name {
            fn code(&self) -> u32 {
                ffier::FfiError::code(self)
            }
            fn message(&self, writer: &mut impl ffier::PushStr) {
                use core::fmt::Write;
                let writer: &mut dyn ffier::PushStr = writer;
                let _ = write!(writer, "{}", self);
            }
        }

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            #reexport_helpers
            (@on_library_export, $type_tag:expr, [$($__handle:ident),*], [$($__value:ident),*]) => {
                impl FfiHandle for #name {
                    const C_HANDLE_NAME: &'static str = stringify!(#name);
                    const TYPE_TAG: u32 = $type_tag;
                }
                impl FfiType for #name {
                    type CRepr = *mut core::ffi::c_void;
                    const C_TYPE_NAME: &'static str = stringify!(#name);
                    const IS_HANDLE: bool = true;
                    fn into_c(self) -> *mut core::ffi::c_void {
                        ffier::ffier_handle_new($type_tag, self)
                    }
                    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {
                        unsafe { ffier::ffier_handle_consume::<Self>(repr) }
                    }
                }
            };
            // Tagged invocation (from library_definition! shim): includes type_tag
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exported_error,
                    name = #name,
                    path = (#error_path),
                    prefix = $prefix,
                    type_tag = $type_tag,
                    variants = [#(#variant_meta_tokens),*],
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
    };

    output.into()
}

pub(crate) struct FfierVariantAttrs {
    pub(crate) code: u32,
    pub(crate) message: Option<String>,
    /// When true, the variant's field (if any) is not marshalled across FFI.
    /// Payload access panics; metadata keeps an opaque entry for each field.
    /// Use this for fields whose types don't implement `FfiType`
    /// (e.g. `anyhow::Error`).
    pub(crate) opaque: bool,
}

pub(crate) fn parse_ffier_error_field(
    field: &syn::Field,
    variant_opaque: bool,
) -> syn::Result<MetaErrorField> {
    let mut result = if variant_opaque {
        MetaErrorField::Opaque
    } else {
        MetaErrorField::Typed(field.ty.to_token_stream())
    };
    for raw_attr in &field.attrs {
        let Some(attr) = extract_ffier_attr(raw_attr) else {
            continue;
        };
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("opaque") {
                result = MetaErrorField::Opaque;
                Ok(())
            } else {
                Err(meta.error("expected `opaque`"))
            }
        })?;
    }
    Ok(result)
}

pub(crate) fn parse_ffier_variant_attrs(
    attrs: &[syn::Attribute],
) -> syn::Result<FfierVariantAttrs> {
    for raw_attr in attrs {
        let Some(attr) = extract_ffier_attr(raw_attr) else {
            continue;
        };
        let attr = &attr;

        let mut code = None;
        let mut message = None;
        let mut opaque = false;

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("code") {
                let value = meta.value()?;
                let lit: syn::LitInt = value.parse()?;
                code = Some(lit.base10_parse::<u32>()?);
                Ok(())
            } else if meta.path.is_ident("message") {
                let value = meta.value()?;
                let lit: LitStr = value.parse()?;
                message = Some(lit.value());
                Ok(())
            } else if meta.path.is_ident("opaque") {
                opaque = true;
                Ok(())
            } else {
                Err(meta.error("expected `code`, `message`, or `opaque`"))
            }
        })?;

        let code = code
            .ok_or_else(|| syn::Error::new_spanned(attr, "missing `code` in #[ffier(code = N)]"))?;

        return Ok(FfierVariantAttrs {
            code,
            message,
            opaque,
        });
    }

    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        "missing #[ffier(code = N)] attribute on variant",
    ))
}
