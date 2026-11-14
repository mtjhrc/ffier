//! Bridge generation from batched metadata.
//!
//! This module coordinates the item emitters and schema output. Sibling modules
//! handle C signatures, parameter/return conversion, and trait dispatch.

use std::collections::{HashMap, HashSet};

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use crate::meta::{
    MetaBitflags, MetaEnum, MetaError, MetaExportable, MetaFreeFunction, MetaImplementable,
    MetaMethod, MetaParamKind, MetaTraitImpl, MetaValueStruct, cfg_predicate_attr, peek_meta_field,
    peek_meta_tag,
};

mod builtins;
mod dispatch;
mod errors;
mod export;
mod generate_bridge;
mod params;
mod returns;
mod schema;
mod signature;

pub(super) use generate_bridge::generate_bridge;

use builtins::{generate_free_object_array, generate_str_free};
use dispatch::{ErrorInfo, ErrorMap, TraitMap, build_trait_map, generate_self_dispatch_bridge};
use errors::{
    generate_error_bridge, generate_error_factory, generate_error_getters, generate_strerror_bridge,
};
use export::{
    generate_exportable_bridge, generate_free_fn_bridge, generate_implementable_bridge,
    generate_trait_impl_bridge,
};
use schema::emit_json;

/// Wrap a multi-item token stream in `#[cfg(PRED)] const _: () = { ... };`
/// so all items are gated as a unit. Returns `code` unchanged if `cfg_attr`
/// is empty (no cfg predicate).
fn wrap_cfg(cfg_attr: Option<TokenStream2>, code: TokenStream2) -> TokenStream2 {
    match cfg_attr {
        Some(attr) => quote! { #attr const _: () = { #code }; },
        None => code,
    }
}

fn generate_one(
    item: TokenStream2,
    trait_map: &TraitMap,
    error_map: &ErrorMap,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    let tag = peek_meta_tag(&item);
    match tag.as_str() {
        "exported_impl" => {
            let meta: MetaExportable = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            let cfg_attr = cfg_predicate_attr(meta.cfg_predicate.as_ref());
            let code = generate_exportable_bridge(
                meta,
                trait_map,
                error_map,
                handle_types,
                value_types,
                lib_crate,
            );
            wrap_cfg(cfg_attr, code)
        }
        "exported_error" => {
            let meta: MetaError = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_error_bridge(meta, lib_crate)
        }
        "exported_trait" => {
            let meta: MetaImplementable = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            let cfg_attr = cfg_predicate_attr(meta.cfg_predicate.as_ref());
            let code = generate_implementable_bridge(meta, lib_crate);
            wrap_cfg(cfg_attr, code)
        }
        "exported_trait_impl" => {
            let meta: MetaTraitImpl = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            let cfg_attr = cfg_predicate_attr(meta.cfg_predicate.as_ref());
            let code = generate_trait_impl_bridge(
                meta,
                trait_map,
                error_map,
                handle_types,
                value_types,
                lib_crate,
            );
            wrap_cfg(cfg_attr, code)
        }
        "exported_enum" | "exported_bitflags" | "exported_value" => {
            // No bridge code needed — enums, bitflags, and value structs are
            // plain values passed inline or by pointer.
            quote! {}
        }
        "exported_fn" => {
            let meta: MetaFreeFunction = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            let cfg_attr = cfg_predicate_attr(meta.cfg_predicate.as_ref());
            let code = generate_free_fn_bridge(
                meta,
                error_map,
                handle_types,
                value_types,
                trait_map,
                lib_crate,
            );
            wrap_cfg(cfg_attr, code)
        }
        _ => {
            let msg = format!("unknown metadata tag `@{tag}`");
            quote! { compile_error!(#msg); }
        }
    }
}

/// Extract a string literal value from a token tree, unwrapping transparent
/// (None-delimited) groups that `macro_rules!` wraps around captured fragments.
/// Uses `syn::LitStr` for proper unescaping.
fn unwrap_literal(tt: Option<proc_macro2::TokenTree>) -> Option<String> {
    match tt {
        Some(proc_macro2::TokenTree::Literal(lit)) => {
            let stream = proc_macro2::TokenStream::from(proc_macro2::TokenTree::Literal(lit));
            syn::parse2::<syn::LitStr>(stream).ok().map(|s| s.value())
        }
        Some(proc_macro2::TokenTree::Group(g)) if g.delimiter() == proc_macro2::Delimiter::None => {
            // Recurse into the transparent group
            unwrap_literal(g.stream().into_iter().next())
        }
        _ => None,
    }
}

/// Generates bridge code from batched metadata items.
///
/// Input: `{ @tag, ... } { @tag, ... } ...` — multiple brace-delimited items.
/// Sorts into errors → exportables → implementables → trait_impls, generates
/// bridge code for each.
pub(super) fn generate_batch_impl(input: TokenStream2) -> TokenStream2 {
    // Parse @lib_crate = path; @primitives_prefix = "..."; from chain macro
    let mut iter = input.into_iter().peekable();
    let lib_crate: TokenStream2 = {
        let mut path_tokens = Vec::new();
        let mut found = false;
        if let Some(proc_macro2::TokenTree::Punct(p)) = iter.peek()
            && p.as_char() == '@'
        {
            found = true;
            iter.next(); // @
            iter.next(); // lib_crate
            iter.next(); // =
            for tt in iter.by_ref() {
                if let proc_macro2::TokenTree::Punct(p) = &tt
                    && p.as_char() == ';'
                {
                    break;
                }
                path_tokens.push(tt);
            }
        }
        if !found {
            return quote! { compile_error!("missing @lib_crate prefix in bridge metadata"); };
        }
        path_tokens.into_iter().collect()
    };
    // Parse optional @-prefixed preamble keys.
    // Known keys: @primitives_prefix, @schema_output.
    let mut primitives_prefix: Option<String> = None;
    let mut schema_output: Option<String> = None;
    while let Some(proc_macro2::TokenTree::Punct(p)) = iter.peek()
        && p.as_char() == '@'
    {
        iter.next(); // @
        let key = match iter.next() {
            Some(proc_macro2::TokenTree::Ident(id)) => id.to_string(),
            _ => {
                return quote! { compile_error!("expected identifier after `@` in preamble"); };
            }
        };
        match iter.next() {
            Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '=' => {}
            _ => {
                let msg = format!("expected `=` after @{key}");
                return quote! { compile_error!(#msg); };
            }
        }
        let value = match unwrap_literal(iter.next()) {
            Some(s) => s,
            None => {
                let msg = format!("expected string literal for @{key}");
                return quote! { compile_error!(#msg); };
            }
        };
        match iter.next() {
            Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == ';' => {}
            _ => {
                let msg = format!("expected `;` after @{key} value");
                return quote! { compile_error!(#msg); };
            }
        }
        match key.as_str() {
            "primitives_prefix" => primitives_prefix = Some(value),
            "schema_output" => schema_output = Some(value),
            _ => {
                let msg = format!("unknown preamble key @{key}");
                return quote! { compile_error!(#msg); };
            }
        }
    }
    let schema_output = match schema_output {
        Some(s) => s,
        None => return quote! { compile_error!("missing @schema_output in bridge metadata"); },
    };

    let mut items: Vec<TokenStream2> = Vec::new();
    for tt in iter {
        if let proc_macro2::TokenTree::Group(g) = tt
            && g.delimiter() == proc_macro2::Delimiter::Brace
        {
            items.push(g.stream());
        }
    }

    // Sort by category for correct ordering
    let mut errors = Vec::new();
    let mut exportables = Vec::new();
    let mut implementables = Vec::new();
    let mut trait_impls = Vec::new();
    let mut enum_constants = Vec::new();
    let mut bitflags_constants = Vec::new();
    let mut value_structs = Vec::new();
    let mut free_fns = Vec::new();

    for item in &items {
        match peek_meta_tag(item).as_str() {
            "exported_error" => errors.push(item.clone()),
            "exported_impl" => exportables.push(item.clone()),
            "exported_trait" => implementables.push(item.clone()),
            "exported_trait_impl" => trait_impls.push(item.clone()),
            "exported_enum" => enum_constants.push(item.clone()),
            "exported_bitflags" => bitflags_constants.push(item.clone()),
            "exported_value" => value_structs.push(item.clone()),
            "exported_fn" => free_fns.push(item.clone()),
            tag => {
                let msg = format!("unknown metadata tag `@{tag}` in batch");
                return quote! { compile_error!(#msg); };
            }
        }
    }

    // Pass 1: Build trait-to-impls map from @trait_impl and @implementable entries.
    // This allows resolving `impl Trait` params automatically.
    let trait_map = build_trait_map(&implementables, &trait_impls);

    let validate_callback_traits = |methods: &[MetaMethod]| -> Option<TokenStream2> {
        for param in methods.iter().flat_map(|method| &method.params) {
            let MetaParamKind::ImplTrait { trait_name, .. } = &param.kind else {
                continue;
            };
            if !trait_map
                .get(trait_name)
                .is_some_and(|info| info.implementable.is_some())
            {
                let message = format!(
                    "ffier: callback trait `{trait_name}` requires a registered exported trait definition to preserve its supertrait and lifetime contracts; annotate the trait and register `trait {trait_name}` in library_definition! (use no_vtable for concrete dispatch only)",
                );
                return Some(quote! { compile_error!(#message); });
            }
        }
        None
    };
    for item in &exportables {
        let meta: MetaExportable = syn::parse2(item.clone()).expect("validated export metadata");
        if let Some(error) = validate_callback_traits(&meta.methods) {
            return error;
        }
    }
    for item in &implementables {
        let meta: MetaImplementable = syn::parse2(item.clone()).expect("validated trait metadata");
        if let Some(error) = validate_callback_traits(&meta.methods) {
            return error;
        }
    }
    for item in &trait_impls {
        let meta: MetaTraitImpl = syn::parse2(item.clone()).expect("validated trait impl metadata");
        if let Some(error) = validate_callback_traits(&meta.methods) {
            return error;
        }
    }
    for item in &free_fns {
        let meta: MetaFreeFunction =
            syn::parse2(item.clone()).expect("validated function metadata");
        if let Some(error) = validate_callback_traits(&meta.methods) {
            return error;
        }
    }

    // Build error map: error name → (type_tag, path)
    let error_map: ErrorMap = {
        let mut map = ErrorMap::new();
        for item in &errors {
            if let Ok(meta) = syn::parse2::<MetaError>(item.clone()) {
                map.insert(
                    meta.name.to_string(),
                    ErrorInfo {
                        type_tag: meta.type_tag,
                    },
                );
            }
        }
        map
    };

    // Build handle set: type names that are opaque handles (exportables + implementables).
    // Used to determine handle-or-null returns for Result<Handle, E>.
    let handle_types: HashSet<String> = {
        let mut set = HashSet::new();
        for item in &exportables {
            if let Ok(meta) = syn::parse2::<MetaExportable>(item.clone()) {
                set.insert(meta.struct_name.to_string());
            }
        }
        for item in &implementables {
            if let Ok(meta) = syn::parse2::<MetaImplementable>(item.clone()) {
                // Vtable wrapper types are also handles
                set.insert(format!("Vtable{}", meta.trait_name));
            }
        }
        set
    };

    // Names of exported value structs. Used to classify `Option<ValueStruct>`
    // returns and to reject value-struct uses the ABI cannot represent.
    let value_types: HashSet<String> = value_structs
        .iter()
        .filter_map(|item| syn::parse2::<MetaValueStruct>(item.clone()).ok())
        .map(|meta| meta.name.to_string())
        .collect();

    // Pass 1.5: Validate value-struct fields. Field types must already have a
    // stable C representation: scalars, exported enums, exported bitflags, or
    // other exported value structs. Anything else is a compile error, since the
    // Rust definition is used as the C layout directly.
    {
        let value_names = &value_types;
        let enum_names: HashSet<String> = enum_constants
            .iter()
            .filter_map(|item| syn::parse2::<MetaEnum>(item.clone()).ok())
            .map(|meta| meta.name.to_string())
            .collect();
        let bitflags_names: HashSet<String> = bitflags_constants
            .iter()
            .filter_map(|item| syn::parse2::<MetaBitflags>(item.clone()).ok())
            .map(|meta| meta.name.to_string())
            .collect();
        const FFI_SCALARS: &[&str] = &[
            "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "isize", "usize",
            "bool",
        ];

        for item in &value_structs {
            let meta: MetaValueStruct = match syn::parse2(item.clone()) {
                Ok(meta) => meta,
                Err(error) => return error.to_compile_error(),
            };
            for field in &meta.fields {
                let ty: syn::Type = match syn::parse2(field.rust_type.clone()) {
                    Ok(ty) => ty,
                    Err(error) => return error.to_compile_error(),
                };
                let type_name = match &ty {
                    syn::Type::Path(path) => path
                        .path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string())
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let supported = FFI_SCALARS.contains(&type_name.as_str())
                    || enum_names.contains(&type_name)
                    || bitflags_names.contains(&type_name)
                    || value_names.contains(&type_name);
                if !supported {
                    let struct_name = meta.name.to_string();
                    let field_name = field.name.to_string();
                    let msg = format!(
                        "ffier value struct `{struct_name}` field `{field_name}` has unsupported \
                         type `{type_name}`; expected an FFI scalar or a registered enum, \
                         bitflags, or value struct"
                    );
                    return quote! { compile_error!(#msg); };
                }
            }
        }
    }

    // Pass 1.5: Validate type tags — check for missing (tag=0) and duplicates.
    // Also builds a tag→name map used to generate __ffier_type_name() for
    // human-readable panic messages on type mismatch.
    let mut tag_to_name: HashMap<u32, String> = HashMap::new();
    {
        let mut check_tag = |tag: u32, name: &str, hint: &str| -> Option<TokenStream2> {
            if tag == 0 {
                let msg =
                    format!("type `{name}` has no type tag; add `{hint}` in library_definition!()");
                return Some(quote! { compile_error!(#msg); });
            }
            if let Some(prev) = tag_to_name.get(&tag) {
                let msg = format!("duplicate type tag {tag}: used by both `{prev}` and `{name}`");
                return Some(quote! { compile_error!(#msg); });
            }
            tag_to_name.insert(tag, name.to_string());
            None
        };

        for item in &errors {
            let meta: MetaError = match syn::parse2(item.clone()) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            let name = meta.name.to_string();
            if let Some(err) = check_tag(meta.type_tag, &name, &format!("{name} = N")) {
                return err;
            }
        }

        for item in &exportables {
            let meta: MetaExportable = match syn::parse2(item.clone()) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            let name = meta.struct_name.to_string();
            if let Some(err) = check_tag(meta.type_tag, &name, &format!("{name} = N")) {
                return err;
            }
        }

        for item in &implementables {
            let meta: MetaImplementable = match syn::parse2(item.clone()) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            let name = format!("Vtable{}", meta.trait_name);
            if let Some(err) = check_tag(
                meta.type_tag,
                &name,
                &format!("trait {} = N", meta.trait_name),
            ) {
                return err;
            }
        }
    }

    // Generate __ffier_type_name(), __ffier_dispatch_panic(), and per-trait
    // accepted-types constants. These are used by all dispatch error messages
    // (self-dispatch, impl Trait, type assertions) for consistent, human-readable
    // panic messages.
    let dispatch_helpers = {
        let mut sorted: Vec<_> = tag_to_name.iter().collect();
        sorted.sort_by_key(|(tag, _)| *tag);
        let tags: Vec<u32> = sorted.iter().map(|(t, _)| **t).collect();
        let names: Vec<&str> = sorted.iter().map(|(_, n)| n.as_str()).collect();

        // Generate one const per trait: __FFIER_ACCEPTED_Fruit = "Apple | Orange | VtableFruit"
        let accepted_consts: Vec<TokenStream2> = trait_map
            .iter()
            .map(|(trait_name, info)| {
                let const_name = format_ident!("__FFIER_ACCEPTED_{trait_name}");
                let accepted = info
                    .variants
                    .iter()
                    .map(|v| v.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ");
                quote! {
                    const #const_name: &str = #accepted;
                }
            })
            .collect();

        quote! {
            /// Look up the type name for a given type tag. Returns "unknown" for
            /// unrecognized tags (e.g. corrupted handle or use-after-free).
            fn __ffier_type_name(tag: u32) -> &'static str {
                match tag {
                    #(#tags => #names,)*
                    _ => "unknown",
                }
            }

            /// Panic with a clear message when a handle's type tag doesn't match
            /// any expected type.
            ///
            /// - `fn_name`: the C function name (e.g. "ft_fruit_value")
            /// - `expected`: what was expected (e.g. "Fruit implementor")
            /// - `accepted`: list of accepted type names (e.g. "Apple | Orange | VtableFruit"),
            ///   or empty string if not applicable (e.g. for single-type assertions)
            /// - `actual_tag`: the type tag read from the handle
            #[cold]
            #[inline(never)]
            fn __ffier_dispatch_panic(
                fn_name: &str,
                expected: &str,
                accepted: &str,
                actual_tag: u32,
            ) -> ! {
                let actual_name = __ffier_type_name(actual_tag);
                let actual_lib = ffier::extract_library_tag(actual_tag);
                // Detect cross-library handle confusion: if the library_tag
                // doesn't match any known type in this library, the handle
                // likely belongs to a different ffier library.
                let cross_lib_hint = if actual_name == "unknown" && actual_lib != 0 {
                    " (handle appears to belong to a different ffier library)"
                } else {
                    ""
                };
                if accepted.is_empty() {
                    panic!(
                        "{}(): expected {}, got {} (type_tag=0x{:08x}){}",
                        fn_name, expected, actual_name, actual_tag, cross_lib_hint,
                    );
                } else {
                    panic!(
                        "{}(): expected {} ({}), got {} (type_tag=0x{:08x}){}",
                        fn_name, expected, accepted, actual_name, actual_tag, cross_lib_hint,
                    );
                }
            }

            #(#accepted_consts)*
        }
    };

    // Pass 2: Generate bridge code for each item in sorted order
    let mut all_code = Vec::new();

    for item in errors
        .iter()
        .chain(exportables.iter())
        .chain(implementables.iter())
        .chain(trait_impls.iter())
        .chain(enum_constants.iter())
        .chain(bitflags_constants.iter())
        .chain(value_structs.iter())
        .chain(free_fns.iter())
    {
        all_code.push(generate_one(
            item.clone(),
            &trait_map,
            &error_map,
            &handle_types,
            &value_types,
            &lib_crate,
        ));
    }

    // Extract prefix from any item for shared types (all items share the same prefix)
    let first_prefix = errors
        .iter()
        .chain(exportables.iter())
        .chain(implementables.iter())
        .chain(trait_impls.iter())
        .chain(enum_constants.iter())
        .chain(bitflags_constants.iter())
        .chain(value_structs.iter())
        .chain(free_fns.iter())
        .next()
        .map(|item| peek_meta_field(item, "prefix"))
        .unwrap_or_default();

    // Debug utility: exported function to inspect a handle's type tag and name.
    let debug_fn = {
        let debug_fn_name = format_ident!("{first_prefix}_debug_handle_type");
        quote! {
            /// Debug utility: read the type tag from a handle and return the
            /// type name as a string. Returns "null" for null handles,
            /// "unknown" for unrecognized tags.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn #debug_fn_name(
                handle: *const core::ffi::c_void,
            ) -> ffier::FfierBytes {
                if handle.is_null() {
                    return unsafe { ffier::FfierBytes::from_str("null") };
                }
                let tag = unsafe { ffier::handle_type_tag(handle) };
                let name = __ffier_type_name(tag);
                unsafe { ffier::FfierBytes::from_str(name) }
            }
        }
    };

    // Owned vtable handle constructor for C callers: owned parameters accept
    // vtable-based objects, and the handle allocation must come from this
    // library so its allocator matches on free.
    let vtable_handle_new_fn = {
        let ctor_name = format_ident!("{first_prefix}_vtable_handle_new");
        quote! {
            /// Allocate an owned vtable handle from this library's allocator.
            ///
            /// The only sound way for non-Rust callers to create a handle for
            /// an owned parameter accepting a vtable-based object. The handle
            /// allocation is made by this library, so its consume and destroy
            /// paths may free it. Stack-allocated or foreign-heap vtable
            /// handles must only be passed to borrowed parameters.
            ///
            /// The vtable's `drop` callback (slot 0, if present) releases
            /// `user_data`; it must not free the handle itself.
            ///
            /// # Safety
            /// - `vtable` must point to a valid vtable for the trait matching
            ///   `type_tag`, starting with its `VtableHeader`.
            /// - `user_data` must stay valid until the vtable `drop` callback
            ///   runs.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn #ctor_name(
                type_tag: u32,
                vtable: *const core::ffi::c_void,
                user_data: *const core::ffi::c_void,
            ) -> *mut core::ffi::c_void {
                ffier::ffier_handle_new(
                    type_tag,
                    ffier::VtableHandle {
                        vtable_ptr: vtable,
                        user_data,
                    },
                )
            }
        }
    };

    // Pass 3: Generate self-dispatch functions for exported traits.
    // For each trait with an @implementable entry, generate per-trait dispatching
    // C functions that read the type tag and dispatch to the concrete implementor.
    for (trait_name, info) in &trait_map {
        if let Some(impl_info) = &info.implementable {
            let cfg_attr = cfg_predicate_attr(impl_info.cfg_predicate.as_ref());
            let code = generate_self_dispatch_bridge(
                trait_name,
                info,
                &first_prefix,
                &trait_map,
                &error_map,
                &handle_types,
                &value_types,
                &lib_crate,
            );
            all_code.push(wrap_cfg(cfg_attr, code));
        }
    }

    // Generate result names and static messages from packed result codes.
    let strerror_fn = generate_strerror_bridge(&first_prefix, &errors, &lib_crate);

    // Generate per-variant field getters for data-carrying error variants
    let error_getters = generate_error_getters(&first_prefix, &errors, &lib_crate);
    let error_factory = generate_error_factory(&first_prefix, &errors, &lib_crate);

    // Generate str_free function for dropping owned strings (Box<str>)
    let str_free_fn = generate_str_free(&first_prefix);

    // Generate free_object_array function
    let free_object_array_fn = generate_free_object_array(&first_prefix);

    // Emit JSON schema to the user-specified path.
    if let Err(error) = emit_json(
        &first_prefix,
        primitives_prefix.as_deref(),
        &schema_output,
        &errors,
        &exportables,
        &implementables,
        &trait_impls,
        &enum_constants,
        &bitflags_constants,
        &value_structs,
        &free_fns,
    ) {
        return error.to_compile_error();
    }

    quote! {
        #dispatch_helpers

        #debug_fn

        #vtable_handle_new_fn

        #(#all_code)*

        #strerror_fn

        #error_getters

        #error_factory

        #str_free_fn

        #free_object_array_fn
    }
}
