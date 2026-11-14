//! Export fixed-layout value structs.

use std::sync::atomic::Ordering;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{ItemStruct, Type};

use super::ExportCfg;
use crate::{
    MACRO_COUNTER, alias::AliasContext, attrs::has_copy_derive, library::value_meta_alias,
    meta::camel_to_snake, reexport::reexport_helpers, types::is_option,
};

/// Export a `#[repr(C)]` struct as a fixed-layout value type.
///
/// Value structs cross the ABI inline or by typed pointer. The Rust
/// definition *is* the C layout, so no wire struct or layout check is
/// generated: the only requirements are `#[repr(C)]`, `Copy`, and field types
/// that already have a stable C representation (scalars, exported enums,
/// exported bitflags, other value structs).
pub(super) fn exportable_value_struct(input: ItemStruct, export_cfg: &ExportCfg) -> TokenStream {
    let name = &input.ident;
    if !matches!(input.vis, syn::Visibility::Public(_)) {
        return syn::Error::new_spanned(&input.vis, "ffier value structs must be public")
            .to_compile_error()
            .into();
    }
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return syn::Error::new_spanned(
            &input.generics,
            "ffier value structs cannot be generic; references and lifetimes are not supported",
        )
        .to_compile_error()
        .into();
    }

    let mut has_repr_c = false;
    for attr in &input.attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        let result = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("C") {
                has_repr_c = true;
                Ok(())
            } else {
                Err(meta.error(
                    "ffier value structs support only #[repr(C)]; packed, aligned, and \
                     integer reprs are not supported",
                ))
            }
        });
        if let Err(error) = result {
            return error.to_compile_error().into();
        }
    }
    if !has_repr_c {
        return syn::Error::new_spanned(name, "ffier value structs require #[repr(C)]")
            .to_compile_error()
            .into();
    }

    // `Copy` keeps the by-value ABI free of drop glue and lets generated code
    // treat the struct as plain bytes on both sides of the boundary.
    if !has_copy_derive(&input.attrs) {
        return syn::Error::new_spanned(
            name,
            "ffier value structs must derive `Copy` (add `#[derive(Clone, Copy)]`)",
        )
        .to_compile_error()
        .into();
    }

    let syn::Fields::Named(fields) = &input.fields else {
        return syn::Error::new_spanned(&input.fields, "ffier value structs require named fields")
            .to_compile_error()
            .into();
    };
    if fields.named.is_empty() {
        return syn::Error::new_spanned(
            &input.fields,
            "ffier value structs cannot be empty; C does not allow empty structs",
        )
        .to_compile_error()
        .into();
    }

    let value_snake = camel_to_snake(&name.to_string());
    let helper_mod_name = format_ident!("_ffier_value_{value_snake}");
    let mut ctx = AliasContext::new(helper_mod_name.clone());
    let mut field_meta = Vec::new();
    let mut field_names = Vec::new();
    let mut field_types = Vec::new();
    for field in &fields.named {
        if let Some(attr) = field
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        {
            return syn::Error::new_spanned(
                attr,
                "field-level cfg on ffier value structs is not supported; \
                 gate the whole struct with ffier::export(cfg = \"...\")",
            )
            .to_compile_error()
            .into();
        }
        if !matches!(field.vis, syn::Visibility::Public(_)) {
            return syn::Error::new_spanned(field, "ffier value-struct fields must be public")
                .to_compile_error()
                .into();
        }
        let field_name = field.ident.as_ref().expect("named field");
        if field_name.to_string().starts_with("r#") {
            return syn::Error::new_spanned(
                field_name,
                "ffier value-struct fields cannot use raw identifiers; C has no `r#`",
            )
            .to_compile_error()
            .into();
        }
        let field_ty = &field.ty;
        if let Type::Path(tp) = field_ty
            && is_option(tp)
        {
            return syn::Error::new_spanned(
                field_ty,
                "ffier value-struct fields cannot be Option; use a sentinel scalar value",
            )
            .to_compile_error()
            .into();
        }
        field_meta.push(quote! { { name = #field_name, rust_type = (#field_ty), } });
        field_names.push(field_name);
        field_types.push(ctx.alias_tokens(field_ty));
    }

    // Resolve the struct at its definition site without requiring a root re-export.
    let value_type = ctx.alias_tokens(&syn::parse_quote!(#name));
    let local_type_aliases = ctx.local_type_aliases();
    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_value_{value_snake}_{counter}");
    let meta_alias_name = value_meta_alias(&syn::parse_quote!(#name));
    let cfg_attr = export_cfg.cfg_attr();
    let cfg_pred_tokens = export_cfg.meta_tokens();

    let (reexport_helpers, reexport_bundle) = reexport_helpers(&[&helper_mod_name], &cfg_attr);

    quote! {
        #reexport_bundle

        #cfg_attr
        #input

        #cfg_attr
        #[doc(hidden)]
        pub mod #helper_mod_name {
            #(#local_type_aliases)*
        }

        // Field `rust_type` tokens are schema data, not paths resolved at the
        // call site, so they are only ever read by the metadata callback.
        #[doc(hidden)]
        #[macro_export]
        #[allow(clippy::crate_in_macro_def)]
        macro_rules! #internal_macro_name {
            #reexport_helpers
            (@on_library_export, $__type_tag:expr, [$($__handle:ident),*], [$($__value:ident),*]) => {
                // The Rust definition is the C layout: value structs are
                // `#[repr(C)]` and all field types already have a C
                // representation, so no conversion is needed.
                impl FfiType for #value_type {
                    type CRepr = Self;
                    const C_TYPE_NAME: &'static str = stringify!(#name);
                    const IS_HANDLE: bool = false;
                    fn into_c(self) -> Self { self }
                    unsafe fn from_c(repr: Self) -> Self { repr }
                }

                impl FfiType for &#value_type {
                    type CRepr = *const #value_type;
                    const C_TYPE_NAME: &'static str = stringify!(#name);
                    const IS_HANDLE: bool = false;
                    fn into_c(self) -> *const #value_type { self }
                    unsafe fn from_c(repr: *const #value_type) -> Self {
                        unsafe { &*repr }
                    }
                }

                impl FfiType for &mut #value_type {
                    type CRepr = *mut #value_type;
                    const C_TYPE_NAME: &'static str = stringify!(#name);
                    const IS_HANDLE: bool = false;
                    fn into_c(self) -> *mut #value_type { self }
                    unsafe fn from_c(repr: *mut #value_type) -> Self {
                        unsafe { &mut *repr }
                    }
                }

                // Optional borrowed values are nullable pointers. The null
                // check is explicit rather than relying on the null pointer
                // optimization; both compile to the same code.
                impl FfiType for Option<&#value_type> {
                    type CRepr = *const #value_type;
                    const C_TYPE_NAME: &'static str = stringify!(#name);
                    const IS_HANDLE: bool = false;
                    fn into_c(self) -> *const #value_type {
                        match self {
                            Some(value) => value,
                            None => core::ptr::null(),
                        }
                    }
                    unsafe fn from_c(repr: *const #value_type) -> Self {
                        if repr.is_null() {
                            None
                        } else {
                            Some(unsafe { &*repr })
                        }
                    }
                }

                impl FfiType for Option<&mut #value_type> {
                    type CRepr = *mut #value_type;
                    const C_TYPE_NAME: &'static str = stringify!(#name);
                    const IS_HANDLE: bool = false;
                    fn into_c(self) -> *mut #value_type {
                        match self {
                            Some(value) => value,
                            None => core::ptr::null_mut(),
                        }
                    }
                    unsafe fn from_c(repr: *mut #value_type) -> Self {
                        if repr.is_null() {
                            None
                        } else {
                            Some(unsafe { &mut *repr })
                        }
                    }
                }
            };
            ($prefix:literal, $__type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exported_value,
                    name = #name,
                    prefix = $prefix,
                    fields = [#(#field_meta),*],
                    #cfg_pred_tokens
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
    }
    .into()
}
