//! Bridge code generation from parsed metadata.
//!
//! `generate_batch_impl` takes batched metadata token streams and produces
//! the corresponding `extern "C"` FFI functions plus a unified `__ffier_header()`
//! function.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use std::collections::{HashMap, HashSet};

use ffier_meta::{
    HasPrefix, MetaError, MetaExportable, MetaImplementable, MetaMethod, MetaMethodContext,
    MetaParamKind, MetaReceiver, MetaReturn, MetaTraitImpl, MetaTypePair, camel_to_snake,
    camel_to_upper_snake, erase_lifetimes_tokens, peek_meta_field, peek_meta_name, peek_meta_tag,
};

/// Maps trait names to their concrete dispatch variants.
pub type TraitMap = HashMap<String, TraitDispatchInfo>;

/// Maps error type names to their metadata (type tag, path, variants).
///
/// Used by exportable bridge generation to emit `ffier_result_from_err`
/// with the correct type tag for `Result<T, E>` returns.
pub type ErrorMap = HashMap<String, ErrorInfo>;

pub struct ErrorInfo {
    pub type_tag: u32,
    pub path: TokenStream2,
}

pub struct TraitDispatchInfo {
    pub variants: Vec<TraitVariant>,
    /// If the trait is `#[implementable]`, the wrapper type path and vtable struct path.
    pub implementable: Option<ImplementableInfo>,
}

pub struct TraitVariant {
    pub name: String,
    pub bridge_type: TokenStream2,
}

pub struct ImplementableInfo {
    pub trait_path: TokenStream2,
    pub wrapper_path: TokenStream2,
    pub vtable_struct_path: TokenStream2,
    pub methods: Vec<MetaMethod>,
    /// Number of methods that belong to this trait (not supertrait methods).
    /// Only the first `own_method_count` methods are dispatched in self-dispatch
    /// functions. Supertrait methods need separate dispatch through their own trait.
    pub own_method_count: usize,
}

/// Build the trait-to-impls map from parsed implementable and trait_impl metadata.
fn build_trait_map(implementables: &[TokenStream2], trait_impls: &[TokenStream2]) -> TraitMap {
    let mut map = TraitMap::new();

    // trait_impl entries: "Fruit for Apple" → Apple is a concrete implementor
    for item in trait_impls {
        if let Ok(meta) = syn::parse2::<MetaTraitImpl>(item.clone()) {
            let trait_name = meta.trait_name.to_string();
            let struct_name = meta.struct_name.to_string();
            let struct_path = meta.struct_path;
            map.entry(trait_name)
                .or_insert_with(|| TraitDispatchInfo {
                    variants: Vec::new(),
                    implementable: None,
                })
                .variants
                .push(TraitVariant {
                    name: struct_name,
                    bridge_type: struct_path,
                });
        }
    }

    // implementable entries: "trait Fruit" → adds VtableFruit wrapper + stores vtable info
    for item in implementables {
        if let Ok(meta) = syn::parse2::<MetaImplementable>(item.clone()) {
            let trait_name = meta.trait_name.to_string();
            let wrapper_name = format!("Vtable{trait_name}");
            let wrapper_path = meta.wrapper_name.clone();
            let vtable_struct_path = meta.vtable_struct_name.clone();
            let methods = meta.methods;
            let own_method_count = meta.own_method_count;

            let info = map.entry(trait_name).or_insert_with(|| TraitDispatchInfo {
                variants: Vec::new(),
                implementable: None,
            });
            info.variants.push(TraitVariant {
                name: wrapper_name,
                bridge_type: wrapper_path.clone(),
            });
            info.implementable = Some(ImplementableInfo {
                trait_path: meta.trait_path,
                wrapper_path,
                vtable_struct_path,
                methods,
                own_method_count,
            });
        }
    }

    map
}

fn generate_one(
    item: TokenStream2,
    trait_map: &TraitMap,
    error_map: &ErrorMap,
    handle_types: &HashSet<String>,
) -> TokenStream2 {
    let tag = peek_meta_tag(&item);
    match tag.as_str() {
        "exportable" => {
            let meta: MetaExportable = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_exportable_bridge(meta, trait_map, error_map, handle_types)
        }
        "error" => {
            let meta: MetaError = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_error_bridge(meta)
        }
        "implementable" => {
            let meta: MetaImplementable = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_implementable_bridge(meta)
        }
        "trait_impl" => {
            let meta: MetaTraitImpl = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_trait_impl_bridge(meta, trait_map)
        }
        _ => {
            let msg = format!("unknown metadata tag `@{tag}`");
            quote! { compile_error!(#msg); }
        }
    }
}

/// Generates bridge code from batched metadata items.
///
/// Input: `{ @tag, ... } { @tag, ... } ...` — multiple brace-delimited items.
/// Sorts into errors → exportables → implementables → trait_impls, generates
/// bridge code for each, and emits a unified `__ffier_header()` function.
pub fn generate_batch_impl(input: TokenStream2) -> TokenStream2 {
    // Split input into individual brace groups
    let mut items: Vec<TokenStream2> = Vec::new();
    for tt in input {
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

    for item in &items {
        match peek_meta_tag(item).as_str() {
            "error" => errors.push(item.clone()),
            "exportable" => exportables.push(item.clone()),
            "implementable" => implementables.push(item.clone()),
            "trait_impl" => trait_impls.push(item.clone()),
            tag => {
                let msg = format!("unknown metadata tag `@{tag}` in batch");
                return quote! { compile_error!(#msg); };
            }
        }
    }

    // Pass 1: Build trait-to-impls map from trait_impl and implementable entries.
    // This allows resolving `impl Trait` params automatically.
    let trait_map = build_trait_map(&implementables, &trait_impls);

    // Build error map: error name → (type_tag, path)
    let error_map: ErrorMap = {
        let mut map = ErrorMap::new();
        for item in &errors {
            if let Ok(meta) = syn::parse2::<MetaError>(item.clone()) {
                map.insert(
                    meta.name.to_string(),
                    ErrorInfo {
                        type_tag: meta.type_tag,
                        path: meta.path.clone(),
                    },
                );
            }
        }
        map
    };

    // Build handle set: type names that are opaque handles (exportables + implementables).
    // Used to determine GLib-style returns for Result<Handle, E>.
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
                if accepted.is_empty() {
                    panic!(
                        "{}(): expected {}, got {} (type_tag={})",
                        fn_name, expected, actual_name, actual_tag,
                    );
                } else {
                    panic!(
                        "{}(): expected {} ({}), got {} (type_tag={})",
                        fn_name, expected, accepted, actual_name, actual_tag,
                    );
                }
            }

            #(#accepted_consts)*
        }
    };

    // Pass 2: Generate bridge code for each item in sorted order
    let mut all_code = Vec::new();
    let mut header_fn_names = Vec::new();

    for item in errors
        .iter()
        .chain(exportables.iter())
        .chain(implementables.iter())
        .chain(trait_impls.iter())
    {
        // Collect header function name before generating
        let name = peek_meta_name(item);
        let tag = peek_meta_tag(item);
        let prefix = peek_meta_field(item, "prefix");
        let fn_pfx = format!("{prefix}_");

        let header_fn = if tag == "implementable" {
            let trait_snake = camel_to_snake(&name);
            format_ident!("{fn_pfx}vtable_{trait_snake}__header")
        } else if tag == "trait_impl" {
            let trait_snake = camel_to_snake(&name);
            let struct_name = peek_meta_field(item, "struct_name");
            let struct_snake = camel_to_snake(&struct_name);
            format_ident!("{fn_pfx}{trait_snake}_for_{struct_snake}__header")
        } else {
            let type_snake = camel_to_snake(&name);
            format_ident!("{fn_pfx}{type_snake}__header")
        };
        header_fn_names.push(header_fn);

        all_code.push(generate_one(item.clone(), &trait_map, &error_map, &handle_types));
    }

    // Extract prefix from any item for shared types (all items share the same prefix)
    let first_prefix = errors
        .iter()
        .chain(exportables.iter())
        .chain(implementables.iter())
        .chain(trait_impls.iter())
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

    // Pass 3: Generate self-dispatch functions for implementable traits.
    // For each trait with an @implementable entry, generate per-trait dispatching
    // C functions that read the type tag and dispatch to the concrete implementor.
    for (trait_name, info) in &trait_map {
        if info.implementable.is_some() {
            let code = generate_self_dispatch_bridge(trait_name, info, &first_prefix, &trait_map);
            all_code.push(code);
            let trait_snake = camel_to_snake(trait_name);
            header_fn_names.push(format_ident!(
                "{first_prefix}_{trait_snake}__dispatch_header"
            ));
        }
    }
    let shared_types_fn = emit_shared_types_fn(&first_prefix);

    // Generate strerror dispatch function — dispatches by type tag to per-error
    // static_message tables.
    let strerror_fn = generate_strerror_bridge(&first_prefix, &errors);

    // Emit JSON metadata to $OUT_DIR/ffier-{prefix}.json
    emit_json(&first_prefix, &errors, &exportables, &implementables, &trait_impls);

    // Generate unified header function
    quote! {
        #dispatch_helpers

        #debug_fn

        #(#all_code)*

        #shared_types_fn

        #strerror_fn

        pub fn __ffier_header(guard: &str) -> ffier_bridge::HeaderBuilder {
            ffier_bridge::HeaderBuilder::new(guard, __ffier_shared_types())
                #(.push(#header_fn_names()))*
                .push(__ffier_strerror_header())
        }
    }
}

/// Emit a function `__ffier_shared_types()` that returns the Str/Bytes/Path
/// typedefs, Result typedef, and macros for the C header. Called once per
/// library, not per type.
fn emit_shared_types_fn(prefix: &str) -> TokenStream2 {
    let type_pfx = ffier_meta::snake_to_pascal(prefix);
    let upper_pfx = format!("{}_", prefix.to_ascii_uppercase());

    let str_c = format!("{type_pfx}Str");
    let bytes_c = format!("{type_pfx}Bytes");
    let path_c = format!("{type_pfx}Path");
    let result_c = format!("{type_pfx}Result");
    let error_c = format!("{type_pfx}Error");
    let result_success = format!("{upper_pfx}RESULT_SUCCESS");
    let str_macro = format!("{upper_pfx}STR");
    let bytes_macro = format!("{upper_pfx}BYTES");

    quote! {
        fn __ffier_shared_types() -> String {
            [
                concat!("typedef uint64_t ", #result_c, ";"),
                concat!("#define ", #result_success, " 0"),
                "",
                concat!("/* Opaque error handle — pass to *_error_message() for details, free with *_error_destroy() */"),
                concat!("typedef void* ", #error_c, ";"),
                "",
                concat!("/* Caller must ensure data is valid UTF-8 */"),
                "typedef struct {",
                "    const char* data;",
                "    size_t len;",
                concat!("} ", #str_c, ";"),
                "",
                concat!("/* Caller must ensure data is a valid UTF-8 path */"),
                concat!("typedef ", #str_c, " ", #path_c, ";"),
                "",
                "typedef struct {",
                "    const uint8_t* data;",
                "    size_t len;",
                concat!("} ", #bytes_c, ";"),
                "",
                concat!("#define ", #str_macro, "(s) ((", #str_c, "){ .data = (s), .len = strlen(s) })"),
                concat!("#if defined(__GNUC__)"),
                concat!("#define ", #bytes_macro, "(arr) ({ \\"),
                concat!("    _Static_assert( \\"),
                concat!("        !__builtin_types_compatible_p(typeof(arr), typeof(&(arr)[0])), \\"),
                concat!("        \"", #bytes_macro, "() requires an array, not a pointer\"); \\"),
                concat!("    ((", #bytes_c, "){ .data = (const uint8_t*)(arr), .len = sizeof(arr) }); \\"),
                "})",
                "#else",
                concat!("#define ", #bytes_macro, "(arr) \\"),
                concat!("    ((", #bytes_c, "){ .data = (const uint8_t*)(arr), .len = sizeof(arr) })"),
                "#endif",
            ].join("\n")
        }
    }
}

// ===========================================================================
// Strerror bridge generation (batch-level, all errors)
// ===========================================================================

/// Generate `{prefix}_result_name()` and `{prefix}_result_name_cstr()`.
///
/// These decode packed `FfierResult` values (type_tag + code) into static
/// variant name strings. Error handle dispatch (destroy, message, code,
/// result) is handled by the Error trait's self-dispatch infrastructure.
fn generate_strerror_bridge(prefix: &str, errors: &[TokenStream2]) -> TokenStream2 {
    let fn_pfx = format!("{prefix}_");
    let type_pfx = ffier_meta::snake_to_pascal(prefix);

    let result_name_cstr_fn = format_ident!("{fn_pfx}result_name_cstr");
    let result_name_fn = format_ident!("{fn_pfx}result_name");
    let result_name_cstr_fn_str = result_name_cstr_fn.to_string();
    let result_name_fn_str = result_name_fn.to_string();

    let result_c_name = format!("{type_pfx}Result");
    let str_c_name = format!("{type_pfx}Str");

    // Build dispatch arms for result_name (packed FfierResult → static CStr)
    let mut result_name_dispatch_arms = Vec::new();
    for item in errors {
        if let Ok(meta) = syn::parse2::<MetaError>(item.clone()) {
            let type_tag = meta.type_tag;
            let path = &meta.path;
            result_name_dispatch_arms.push(quote! {
                #type_tag => {
                    let code = ffier::ffier_result_code(r);
                    <#path as ffier::FfiError>::static_message(code).as_ptr()
                }
            });
        }
    }

    let unknown_msg_bytes = proc_macro2::Literal::byte_string(b"unknown error\0");

    quote! {
        /// Variant name as a null-terminated C string.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #result_name_cstr_fn(
            r: ffier::FfierResult,
        ) -> *const core::ffi::c_char {
            if r == 0 { return b"success\0".as_ptr() as *const core::ffi::c_char; }
            let type_tag = ffier::ffier_result_type_tag(r);
            match type_tag {
                #(#result_name_dispatch_arms)*
                _ => unsafe {
                    core::ffi::CStr::from_bytes_with_nul_unchecked(#unknown_msg_bytes).as_ptr()
                },
            }
        }

        /// Variant name as `FtStr` (length-prefixed, no strlen needed).
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #result_name_fn(
            r: ffier::FfierResult,
        ) -> ffier::FfierBytes {
            let ptr = unsafe { #result_name_cstr_fn(r) };
            let len = unsafe { core::ffi::CStr::from_ptr(ptr) }.to_bytes().len();
            ffier::FfierBytes { data: ptr as *const u8, len }
        }

        fn __ffier_strerror_header() -> ffier_bridge::HeaderSection {
            let mut decls = String::new();
            decls.push_str(&format!(
                "{} {}({} r);\n",
                #str_c_name, #result_name_fn_str, #result_c_name,
            ));
            decls.push_str(&format!(
                "const char* {}({} r);",
                #result_name_cstr_fn_str, #result_c_name,
            ));
            ffier_bridge::HeaderSection {
                struct_name: "error".to_string(),
                handle_typedef: String::new(),
                declarations: decls,
            }
        }
    }
}

// ===========================================================================
// Exportable bridge generation
// ===========================================================================

fn generate_exportable_bridge(
    meta: MetaExportable,
    trait_map: &TraitMap,
    error_map: &ErrorMap,
    handle_types: &HashSet<String>,
) -> TokenStream2 {
    let struct_path = &meta.struct_path;
    let struct_name = &meta.struct_name.to_string();
    let fn_pfx = meta.fn_pfx();
    let type_pfx = meta.type_pfx();
    let handle_c_name = meta.handle_c_name();
    let str_c_name = format!("{type_pfx}Str");

    let struct_lower = camel_to_snake(struct_name);

    let mut ffi_fns = Vec::new();
    let handle_typedef_expr = quote! { concat!("typedef void* ", #handle_c_name, ";") };
    let mut decl_exprs: Vec<TokenStream2> = Vec::new();

    // Generate typedefs for impl Trait dispatch types (resolved from trait map)
    let mut generated_dyn_types: Vec<String> = Vec::new();
    for m in &meta.methods {
        for p in &m.params {
            if let MetaParamKind::ImplTrait { trait_name, .. } = &p.kind {
                let c_name = format!("{type_pfx}{trait_name}");
                if generated_dyn_types.contains(&c_name) {
                    continue;
                }
                generated_dyn_types.push(c_name.clone());

                if let Some(info) = trait_map.get(trait_name) {
                    let variant_names: Vec<String> = info
                        .variants
                        .iter()
                        .map(|v| format!("{type_pfx}{}", v.name))
                        .collect();
                    let variants_comment = variant_names.join(" | ");
                    decl_exprs.push(quote! {
                        format!("typedef void* {}; /* {} */", #c_name, #variants_comment)
                    });
                }
            }
        }
    }

    // Method FFI functions
    for m in &meta.methods {
        let ffi_name_str = format!("{}{}", fn_pfx, m.ffi_name());
        let ffi_name = format_ident!("{}", ffi_name_str);
        let method_name = &m.name;

        let has_receiver = m.receiver != MetaReceiver::None;
        let is_mut = m.receiver == MetaReceiver::Mut;
        let is_by_value = m.receiver == MetaReceiver::Value;
        let is_builder = m.is_builder();

        let handle_type = if is_builder && is_by_value {
            // Builder by-value: C caller passes &handle (pointer-to-pointer).
            format!("{handle_c_name}* handle")
        } else {
            format!("{handle_c_name} handle")
        };

        // Single source of truth: the extern "C" fn signature.
        let c_sig = c_signature_for_method(m, &meta.prefix, SignatureContext::Bridge, handle_types);

        // Self access via borrow/consume (instance methods only).
        //
        // For builder by-value methods, the C param is a pointer-to-pointer
        // (FtConfig* = void**). We dereference it first, storing the slot
        // address for write-back later.
        let obj_binding = if has_receiver {
            let type_assert = quote! {
                let __actual = unsafe { ffier::handle_type_tag(handle) };
                let __expected = <#struct_path as ffier::FfiHandle>::TYPE_TAG;
                if __actual != __expected {
                    __ffier_dispatch_panic(
                        #ffi_name_str,
                        <#struct_path as ffier::FfiHandle>::C_HANDLE_NAME,
                        "",
                        __actual,
                    );
                }
            };
            let deref_slot = if is_builder && is_by_value {
                // `handle` param is *mut c_void but actually void**.
                // Save the slot, read the real handle pointer.
                quote! {
                    let __handle_slot = handle as *mut *mut core::ffi::c_void;
                    let handle = unsafe { *__handle_slot };
                }
            } else {
                quote! {}
            };
            let cast = if is_by_value {
                quote! {
                    ffier::ffier_handle_consume::<#struct_path>(handle)
                }
            } else if is_mut {
                quote! {
                    ffier::ffier_handle_borrow_mut::<#struct_path>(handle)
                }
            } else {
                quote! {
                    ffier::ffier_handle_borrow::<#struct_path>(handle)
                }
            };
            Some(quote! { #deref_slot #type_assert let obj = unsafe { #cast }; })
        } else {
            None
        };

        let mut c_type_exprs = Vec::new();
        let mut header_param_names: Vec<String> = Vec::new();
        for p in &m.params {
            let name = p.name.to_string();
            if matches!(p.kind, MetaParamKind::StrSlice) {
                let ptr_type = format!("const {str_c_name}*");
                c_type_exprs.push(quote! { #ptr_type });
                header_param_names.push(name.clone());
                c_type_exprs.push(quote! { "uintptr_t" });
                header_param_names.push(format!("{name}_len"));
            } else {
                c_type_exprs.push(meta_param_c_type_expr(&p.kind, &type_pfx));
                header_param_names.push(name);
            }
        }
        let param_name_str_refs: Vec<_> = header_param_names.iter().collect();

        // Collect all impl Trait params with their dispatch info
        struct ImplTraitParam {
            name: syn::Ident,
            dispatch: ffier_meta::DispatchMode,
            trait_name: String,
            variants: Vec<(String, TokenStream2)>,
        }
        let impl_trait_params: Vec<_> = m
            .params
            .iter()
            .filter_map(|p| {
                  if let MetaParamKind::ImplTrait {
                    trait_name,
                    dispatch,
                    ..
                } = &p.kind
                {
                    trait_map.get(trait_name).map(|info| ImplTraitParam {
                        name: p.name.clone(),
                        dispatch: *dispatch,
                        trait_name: trait_name.clone(),
                        variants: info
                            .variants
                            .iter()
                            .map(|v| (v.name.clone(), v.bridge_type.clone()))
                            .collect(),
                    })
                } else {
                    None
                }
            })
            .collect();

        // Check for dispatch limit (auto mode only)
        let concrete_params: Vec<_> = impl_trait_params
            .iter()
            .filter(|p| p.dispatch != ffier_meta::DispatchMode::Vtable)
            .collect();
        let total_branches: u64 = concrete_params
            .iter()
            .map(|p| p.variants.len() as u64)
            .product();
        if total_branches > ffier_meta::DEFAULT_MAX_DISPATCH
            && impl_trait_params
                .iter()
                .any(|p| p.dispatch == ffier_meta::DispatchMode::Auto)
        {
            let method_name_str = m.name.to_string();
            let msg = format!(
                "ffier: method `{method_name_str}` would generate {total_branches} dispatch \
                 branches (limit: {}). Add `#[ffier(dispatch = vtable)]` to the impl Trait \
                 param(s) or `#[ffier(dispatch = concrete)]` to override the limit.",
                ffier_meta::DEFAULT_MAX_DISPATCH,
            );
            return quote! { compile_error!(#msg); };
        }

        // Check vtable dispatch is possible (trait must be #[implementable])
        for p in &impl_trait_params {
            if p.dispatch == ffier_meta::DispatchMode::Vtable
                && trait_map
                    .get(&p.trait_name)
                    .and_then(|info| info.implementable.as_ref())
                    .is_none()
            {
                let msg = format!(
                    "ffier: `#[ffier(dispatch = vtable)]` on param `{}` requires trait `{}` \
                     to have `#[ffier::implementable]`",
                    p.name, p.trait_name,
                );
                return quote! { compile_error!(#msg); };
            }
        }

        // Pre-bindings for multi-param types
        let mut pre_bindings = Vec::new();
        let converted_args: Vec<_> = m
            .params
            .iter()
            .map(|p| {
                let id = &p.name;
                match &p.kind {
                    MetaParamKind::ImplTrait { .. } => quote! { #id },
                    MetaParamKind::StrSlice => {
                        let len_name = format!("{}_len", p.name);
                        let len_id = &c_sig
                            .params
                            .iter()
                            .find(|cp| cp.name == len_name)
                            .expect("StrSlice must have _len param in c_sig")
                            .name;
                        let binding = meta_param_conversion(id, &p.kind, Some(len_id));
                        let vec_id = format_ident!("__{id}_vec");
                        pre_bindings.push(quote! { let #vec_id = #binding; });
                        quote! { &#vec_id }
                    }
                    other => meta_param_conversion(id, other, None),
                }
            })
            .collect();

        // Build the method call expression
        let base_method_call = if has_receiver {
            quote! { obj.#method_name(#(#converted_args),*) }
        } else {
            quote! { <#struct_path>::#method_name(#(#converted_args),*) }
        };

        // Determine effective dispatch mode for each param.
        // Auto mode: if total branches ≤ limit, all concrete. Otherwise,
        // first auto param stays concrete, rest become vtable.
        let all_concrete = impl_trait_params
            .iter()
            .all(|p| p.dispatch != ffier_meta::DispatchMode::Vtable)
            && (total_branches <= ffier_meta::DEFAULT_MAX_DISPATCH
                || impl_trait_params
                    .iter()
                    .all(|p| p.dispatch == ffier_meta::DispatchMode::Concrete));
        let effective_dispatch: Vec<bool> = if all_concrete {
            // All concrete dispatch
            vec![false; impl_trait_params.len()]
        } else {
            // Hybrid: explicit concrete stays concrete, explicit vtable stays vtable,
            // auto params: first one concrete, rest vtable
            let mut first_auto_seen = false;
            impl_trait_params
                .iter()
                .map(|p| match p.dispatch {
                    ffier_meta::DispatchMode::Concrete => false, // concrete
                    ffier_meta::DispatchMode::Vtable => true,    // vtable
                    ffier_meta::DispatchMode::Auto => {
                        if !first_auto_seen {
                            first_auto_seen = true;
                            false // first auto → concrete
                        } else {
                            true // rest → vtable
                        }
                    }
                })
                .collect()
        };

        // Dynamic dispatch via FfierBoxDyn: wrap each vtable-mode param into
        // FfierBoxDyn<dyn Trait>. Linear in variants (N branches per param).
        let mut vtable_pre_bindings: Vec<TokenStream2> = Vec::new();
        for (i, p) in impl_trait_params.iter().enumerate() {
            if !effective_dispatch[i] {
                continue;
            }
            let dyn_id = &p.name;
            let info = trait_map.get(&p.trait_name).unwrap();

            // Check that the trait has dispatch support (implementable or dispatch)
            // The trait map has implementable info if #[ffier::implementable] was used.
            // For #[ffier::dispatch]-only traits, we don't need implementable info —
            // we just need FfierBoxDyn<dyn Trait> to implement Trait.
            // Either way, the codegen is the same: unbox and wrap in FfierBoxDyn.

            let trait_ident = if let Some(imp) = &info.implementable {
                imp.trait_path.clone()
            } else {
                let ident = format_ident!("{}", p.trait_name);
                quote! { #ident }
            };

            let mut branches = Vec::new();
            for v in &info.variants {
                let ty = &v.bridge_type;
                branches.push(quote! {
                    if __type_tag == <#ty as ffier::FfiHandle>::TYPE_TAG {
                        let __val = unsafe { ffier::ffier_handle_consume::<#ty>(#dyn_id) };
                        ffier::FfierBoxDyn(Box::new(__val) as Box<dyn #trait_ident>)
                    }
                });
            }

            let expected_msg = format!("impl {}", p.trait_name);
            let accepted_const = format_ident!("__FFIER_ACCEPTED_{}", p.trait_name);

            vtable_pre_bindings.push(quote! {
                let #dyn_id: ffier::FfierBoxDyn<dyn #trait_ident> = {
                    let __type_tag = unsafe { ffier::handle_type_tag(#dyn_id) };
                    #(#branches else)* {
                        __ffier_dispatch_panic(#ffi_name_str, #expected_msg, #accepted_const, __type_tag);
                    }
                };
            });
        }

        // Concrete nested dispatch for non-vtable impl Trait params.
        let concrete_impl_trait_params: Vec<_> = impl_trait_params
            .iter()
            .enumerate()
            .filter(|(i, _)| !effective_dispatch[*i])
            .map(|(_, p)| p)
            .collect();
        let method_call =
            concrete_impl_trait_params
                .iter()
                .rev()
                .fold(base_method_call, |inner, p| {
                    let dyn_id = &p.name;
                    let variants = &p.variants;
                    let if_branches: Vec<_> = variants
                        .iter()
                        .map(|(_, ty_tokens)| {
                            quote! {
                                if __type_tag == <#ty_tokens as ffier::FfiHandle>::TYPE_TAG {
                                    let #dyn_id = unsafe { ffier::ffier_handle_consume::<#ty_tokens>(#dyn_id) };
                                    #inner
                                }
                            }
                        })
                        .collect();

                    let expected_msg = format!("impl {}", p.trait_name);
                    let accepted_const = format_ident!("__FFIER_ACCEPTED_{}", p.trait_name);

                    quote! {{
                        let __type_tag = unsafe { ffier::handle_type_tag(#dyn_id) };
                        #(#if_branches else)* {
                            __ffier_dispatch_panic(#ffi_name_str, #expected_msg, #accepted_const, __type_tag);
                        }
                    }}
                });

        // Doxygen comment
        let (has_out_param, err_c_name_for_doc) = match &m.ret {
            MetaReturn::Result { ok, .. } => {
                (ok.is_some(), Some(format!("{type_pfx}Result")))
            }
            _ => (false, None),
        };
        let param_name_strs: Vec<String> = m.params.iter().map(|p| p.name.to_string()).collect();
        if let Some(doc) = build_doxygen_comment(
            &m.doc(),
            &param_name_strs,
            has_out_param,
            err_c_name_for_doc.as_deref(),
        ) {
            decl_exprs.push(quote! { #doc });
        }

        let header_handle = if has_receiver {
            Some(&handle_type)
        } else {
            None
        };

        // Extern fn signature from c_sig (shared across all return variants)
        let sig_names: Vec<_> = c_sig.params.iter().map(|p| &p.name).collect();
        let sig_types: Vec<_> = c_sig.params.iter().map(|p| &p.c_type).collect();
        let sig_ret = &c_sig.ret;

        match &m.ret {
            MetaReturn::Void => {
                let header_line = build_header_line(
                    quote! { "void" },
                    &ffi_name_str,
                    header_handle,
                    &c_type_exprs,
                    &param_name_str_refs,
                    None,
                    false,
                    None,
                );
                decl_exprs.push(header_line);

                let body = if is_builder && is_by_value {
                    // Builder by-value: obj_binding dereferences the double
                    // pointer (saving __handle_slot), consumes the old handle,
                    // calls the method, writes the new pointer back.
                    quote! {
                        #obj_binding
                        #(#vtable_pre_bindings)*
                        #(#pre_bindings)*
                        let result = #method_call;
                        let __new_ptr = <#struct_path as ffier::FfiType>::into_c(result);
                        unsafe { *__handle_slot = __new_ptr };
                    }
                } else {
                    quote! {
                        #obj_binding
                        #(#vtable_pre_bindings)*
                        #(#pre_bindings)*
                        #method_call;
                    }
                };

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        #(#sig_names: #sig_types),*
                    ) #sig_ret {
                        #body
                    }
                });
            }
            MetaReturn::Value(vk) => {
                let bridge_type = &vk.bridge_type;
                // All values (handles and primitives) returned via into_c.
                // Handles → *mut c_void, primitives → their CRepr.
                let ret_c_header = quote! {
                    &ffier_bridge::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
                };
                let header_line = build_header_line(
                    ret_c_header,
                    &ffi_name_str,
                    header_handle,
                    &c_type_exprs,
                    &param_name_str_refs,
                    None,
                    false,
                    None,
                );
                decl_exprs.push(header_line);

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        #(#sig_names: #sig_types),*
                    ) #sig_ret {
                        #obj_binding
                        #(#vtable_pre_bindings)*
                        #(#pre_bindings)*
                        let result = #method_call;
                        <#bridge_type as ffier::FfiType>::into_c(result)
                    }
                });
            }
            MetaReturn::Result { ok, err_ident } => {
                let ok_is_handle = ok.is_some()
                    && is_result_ok_handle(&m.rust_ret, handle_types);

                // Look up the error type's type_tag from the error map
                let err_info = error_map.get(err_ident);
                let err_type_tag = err_info.map(|i| i.type_tag).unwrap_or(0);
                let err_path = err_info.map(|i| &i.path);

                // Error boxing: allocate error as a handle via into_c,
                // write the pointer through err_out (which is *mut *mut c_void).
                let box_expr = if err_path.is_some() {
                    quote! {
                        if !err_out.is_null() {
                            unsafe {
                                *err_out = <_ as ffier::FfiType>::into_c(e);
                            }
                        }
                    }
                } else {
                    quote! {}
                };

                let err_handle_c_name = format!("{type_pfx}Error");

                if ok_is_handle {
                    // GLib-style: return handle directly, NULL on error.
                    let bridge_type = &ok.as_ref().unwrap().bridge_type;
                    let ret_c_header = quote! {
                        &ffier_bridge::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
                    };
                    let header_line = build_header_line(
                        ret_c_header,
                        &ffi_name_str,
                        header_handle,
                        &c_type_exprs,
                        &param_name_str_refs,
                        None,
                        true,
                        Some(&err_handle_c_name),
                    );
                    decl_exprs.push(header_line);

                    let ok_branch = quote! {
                        Ok(ok_val) => {
                            <_ as ffier::FfiType>::into_c(ok_val)
                        }
                    };
                    let err_branch = quote! {
                        Err(e) => {
                            #box_expr
                            core::ptr::null_mut()
                        }
                    };

                    ffi_fns.push(quote! {
                        #[unsafe(no_mangle)]
                        pub unsafe extern "C" fn #ffi_name(
                            #(#sig_names: #sig_types),*
                        ) #sig_ret {
                            #obj_binding
                            #(#vtable_pre_bindings)*
                            #(#pre_bindings)*
                            match #method_call {
                                #ok_branch
                                #err_branch
                            }
                        }
                    });
                } else {
                    // FtResult style: return packed error code, out-params for ok value.
                    let result_c_name = format!("{type_pfx}Result");
                    let ret_c_name = quote! { #result_c_name };

                    let out_c_type = ok.as_ref().map(|vk| {
                        let bridge_type = &vk.bridge_type;
                        quote! {
                            &ffier_bridge::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
                        }
                    });

                    let header_line = build_header_line(
                        ret_c_name,
                        &ffi_name_str,
                        header_handle,
                        &c_type_exprs,
                        &param_name_str_refs,
                        out_c_type.as_ref(),
                        true,
                        Some(&err_handle_c_name),
                    );
                    decl_exprs.push(header_line);

                    let ok_branch = match ok {
                        Some(vk) => {
                            let bridge_type = &vk.bridge_type;
                            quote! {
                                Ok(ok_val) => {
                                    unsafe { result.write(<#bridge_type as ffier::FfiType>::into_c(ok_val)) };
                                    ffier::FFIER_RESULT_SUCCESS
                                }
                            }
                        }
                        None if is_builder && is_by_value => quote! {
                            Ok(new_self) => {
                                let __new_ptr = <#struct_path as ffier::FfiType>::into_c(new_self);
                                unsafe { *__handle_slot = __new_ptr };
                                ffier::FFIER_RESULT_SUCCESS
                            }
                        },
                        None => quote! {
                            Ok(_) => ffier::FFIER_RESULT_SUCCESS,
                        },
                    };

                    let err_branch = if is_builder && is_by_value {
                        // Builder error: the old handle was already consumed.
                        // Write NULL to the caller's variable so they don't use
                        // a dangling pointer.
                        quote! {
                            Err(e) => {
                                let __r = ffier::ffier_result(#err_type_tag, ffier::FfiError::code(&e));
                                unsafe { *__handle_slot = core::ptr::null_mut() };
                                #box_expr
                                __r
                            }
                        }
                    } else {
                        quote! {
                            Err(e) => {
                                let __r = ffier::ffier_result(#err_type_tag, ffier::FfiError::code(&e));
                                #box_expr
                                __r
                            }
                        }
                    };

                    ffi_fns.push(quote! {
                        #[unsafe(no_mangle)]
                        pub unsafe extern "C" fn #ffi_name(
                            #(#sig_names: #sig_types),*
                        ) #sig_ret {
                            #obj_binding
                            #(#vtable_pre_bindings)*
                            #(#pre_bindings)*
                            match #method_call {
                                #ok_branch
                                #err_branch
                            }
                        }
                    });
                }
            }
        }
    }

    // destroy function
    let destroy_name = format_ident!("{fn_pfx}{struct_lower}_destroy");
    let destroy_str = destroy_name.to_string();

    decl_exprs.push(quote! { concat!("void ", #destroy_str, "(", #handle_c_name, " handle);") });

    ffi_fns.push(quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #destroy_name(handle: *mut core::ffi::c_void) {
            if !handle.is_null() {
                let __actual = unsafe { ffier::handle_type_tag(handle) };
                let __expected = <#struct_path as ffier::FfiHandle>::TYPE_TAG;
                if __actual != __expected {
                    __ffier_dispatch_panic(
                        #destroy_str,
                        <#struct_path as ffier::FfiHandle>::C_HANDLE_NAME,
                        "",
                        __actual,
                    );
                }
                unsafe { ffier::ffier_handle_drop::<#struct_path>(handle) };
            }
        }
    });

    // Header function
    let header_fn_name = format_ident!("{fn_pfx}{struct_lower}__header");
    let num_decls = decl_exprs.len();

    quote! {
        #(#ffi_fns)*

        pub fn #header_fn_name() -> ffier_bridge::HeaderSection {
            let handle_typedef = #handle_typedef_expr .to_string();
            let decl_lines: [String; #num_decls] = [
                #(#decl_exprs .to_string()),*
            ];
            let declarations = decl_lines.join("\n");
            ffier_bridge::HeaderSection {
                struct_name: #struct_name.to_string(),
                handle_typedef,
                declarations,
            }
        }
    }
}

// ===========================================================================
// Error bridge generation
// ===========================================================================

fn generate_error_bridge(meta: MetaError) -> TokenStream2 {
    let name = &meta.name;
    let name_str = name.to_string();
    let upper_pfx = meta.upper_pfx();
    let type_tag = meta.type_tag;
    let path = &meta.path;

    // Strip "Error" suffix from type name for constant prefix:
    // TestError → TEST, CalcError → CALC, BufferError → BUFFER
    let stripped_name = name_str
        .strip_suffix("Error")
        .unwrap_or(&name_str);
    let err_upper = camel_to_upper_snake(stripped_name);
    let full_upper_pfx = format!("{upper_pfx}ERROR_{err_upper}");

    let err_snake = camel_to_snake(&name_str);
    let fn_pfx = meta.fn_pfx();
    let header_fn_name = format_ident!("{fn_pfx}{err_snake}__header");

    // No per-type bridge functions needed — strerror is emitted at batch level.
    // Only emit the header section with constants + handle typedef.
    let type_pfx = meta.type_pfx();
    let handle_c_name = format!("{type_pfx}{name_str}");
    quote! {
        pub fn #header_fn_name() -> ffier_bridge::HeaderSection {
            let full_upper_pfx = #full_upper_pfx;

            let mut decls = String::new();

            // Emit constants with baked-in type tags:
            // #define FT_ERROR_TEST_NOT_FOUND ((uint64_t)1 << 32 | 1)
            for (variant_name, code) in <#path as ffier::FfiError>::codes() {
                decls.push_str(&format!(
                    "#define {}_{} ((uint64_t){} << 32 | {})\n",
                    full_upper_pfx, variant_name, #type_tag, code,
                ));
            }

            ffier_bridge::HeaderSection {
                struct_name: #name_str.to_string(),
                handle_typedef: format!("typedef void* {};", #handle_c_name),
                declarations: decls,
            }
        }
    }
}

// ===========================================================================
// Implementable bridge generation
// ===========================================================================

fn generate_implementable_bridge(meta: MetaImplementable) -> TokenStream2 {
    let vtable_c_name = meta.vtable_c_name();
    let type_pfx = meta.type_pfx();
    let fn_pfx = meta.fn_pfx();

    let trait_name_str = meta.trait_name.to_string();
    let trait_snake = camel_to_snake(&trait_name_str);
    let header_fn_name = format_ident!("{fn_pfx}vtable_{trait_snake}__header");
    let vtable_section_name = format!("Vtable{trait_name_str}");

    // Build header lines for vtable struct
    let mut header_lines: Vec<TokenStream2> = Vec::new();

    header_lines.push(quote! { concat!("typedef struct {") });

    // drop function pointer — always first for stable ABI offset
    header_lines.push(quote! { "    void (*drop)(void* self_data);" });

    // Method function pointers, sorted by explicit index with padding for gaps.
    {
        let mut sorted_methods: Vec<_> = meta.methods.iter().collect();
        sorted_methods.sort_by_key(|m| m.index());

        let mut next_slot = 0usize;
        for m in &sorted_methods {
            // raw_handle methods don't occupy vtable slots
            if m.raw_handle() {
                continue;
            }
            while next_slot < m.index() {
                let pad_comment = format!(
                    "    void (*__reserved_{next_slot})(void); /* reserved slot {next_slot} */"
                );
                header_lines.push(quote! { #pad_comment });
                next_slot += 1;
            }
            let name_str = m.name.to_string();
            let (param_id_strs, param_type_exprs) = vtable_param_c_types(&m.params, &type_pfx);
            let ret_c_expr = vtable_ret_c_expr(&m.ret, &type_pfx);

            header_lines.push(quote! {{
                let mut s = String::from("    ");
                s.push_str(#ret_c_expr);
                s.push_str(" (*");
                s.push_str(#name_str);
                s.push_str(")(void* self_data");
                let param_types: &[&str] = &[#(#param_type_exprs),*];
                let param_names: &[&str] = &[#(#param_id_strs),*];
                for (t, n) in param_types.iter().zip(param_names.iter()) {
                    s.push_str(", ");
                    s.push_str(t);
                    s.push(' ');
                    s.push_str(n);
                }
                s.push_str(");");
                s
            }});
            next_slot = m.index() + 1;
        }
        // Pad trailing reserved slots
        while next_slot <= meta.max_vtable_slot {
            let pad_comment = format!(
                "    void (*__reserved_{next_slot})(void); /* reserved slot {next_slot} */"
            );
            header_lines.push(quote! { #pad_comment });
            next_slot += 1;
        }
    }
    header_lines.push(quote! { concat!("} ", #vtable_c_name, ";") });
    header_lines.push(quote! { "" });
    // No from_vtable extern function — C callers use FT_PTR_OBJECT macro.

    let num_header_lines = header_lines.len();

    quote! {
        pub fn #header_fn_name() -> ffier_bridge::HeaderSection {
            let decl_lines: [String; #num_header_lines] = [
                #(#header_lines .to_string()),*
            ];
            let declarations = decl_lines.join("\n");
            ffier_bridge::HeaderSection {
                struct_name: #vtable_section_name.to_string(),
                handle_typedef: String::new(),
                declarations,
            }
        }
    }
}

// ===========================================================================
// Shared C ABI type resolution — used by both ffier-bridge and ffier-gen-rust
// ===========================================================================

/// Extract the last path segment name from a token stream like `$crate::Gadget`.
pub fn last_path_segment(ts: &TokenStream2) -> Option<String> {
    let mut last_ident = None;
    for tt in ts.clone() {
        if let proc_macro2::TokenTree::Ident(id) = tt {
            last_ident = Some(id.to_string());
        }
    }
    last_ident
}

/// Check if a Result<T, E> Ok type is a handle, using the original Rust
/// return type tokens (e.g. `Result<Gadget, TestError>`) rather than the
/// bridge_type alias (which may be an opaque `_TypeN`).
pub fn is_result_ok_handle(rust_ret: &TokenStream2, handle_types: &HashSet<String>) -> bool {
    let ok_type = extract_result_ok_type(rust_ret);
    last_path_segment(&ok_type)
        .map(|name| handle_types.contains(&name))
        .unwrap_or(false)
}

/// Extract the Ok type from `Result<OkType, ErrType>` tokens.
pub fn extract_result_ok_type(tokens: &TokenStream2) -> TokenStream2 {
    if let Ok(ty) = syn::parse2::<syn::Type>(tokens.clone())
        && let syn::Type::Path(tp) = &ty
        && let Some(last) = tp.path.segments.last()
        && last.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(syn::GenericArgument::Type(ok_ty)) = args.args.first()
    {
        return quote! { #ok_ty };
    }
    tokens.clone()
}

/// Context for type resolution in extern fn signatures.
///
/// Both contexts resolve to the same C types via `<T as FfiType>::CRepr`,
/// but use different token streams for the type `T`:
/// - `Bridge`: uses `bridge_type` ($crate:: paths that resolve in cdylib)
/// - `Client`: uses `rust_type` (plain types for standalone source)
///
/// TODO(remove): This enum and the `Client` variant exist solely because
/// `ffier-gen-rust` calls `c_signature_for_method` with `Client` to generate
/// extern declarations. Once `ffier-gen-rust` is ported to consume the JSON
/// schema instead of calling into `ffier-bridge`, delete `SignatureContext`,
/// the `Client` variant, and all `rust_type` branches in `c_param_type` /
/// `c_return_type` / `c_out_param_type`. The bridge should only deal with
/// `bridge_type`.
pub enum SignatureContext {
    /// C bridge in cdylib — types via $crate:: paths
    Bridge,
    /// Standalone Rust client source — types via original names.
    /// TODO(remove): see enum-level doc comment.
    Client,
}

/// A single parameter in a C extern signature.
pub struct CExternParam {
    pub name: syn::Ident,
    pub c_type: TokenStream2,
}

/// Complete C extern function signature for a method.
///
/// Contains all the information needed to emit an `unsafe extern "C" { fn ... }`
/// declaration. The parameters include handle, regular params, and out-param
/// (for Result returns) in the order they appear.
pub struct CExternSignature {
    /// Fully qualified extern function name (e.g. "mylib_calculator_add").
    pub fn_name: String,
    /// All parameters in declaration order.
    pub params: Vec<CExternParam>,
    /// Return type tokens (empty for void).
    pub ret: TokenStream2,
}

/// Compute the full C extern signature for a method.
///
/// This is the single source of truth for "what does this method look like
/// as an `extern "C"` function". Both the C bridge generator and the Rust
/// client generator should agree on this.
///
pub fn c_signature_for_method(
    method: &MetaMethod,
    prefix: &str,
    ctx: SignatureContext,
    handle_types: &HashSet<String>,
) -> CExternSignature {
    let fn_name = format!("{}_{}", prefix, method.ffi_name());
    let mut params = Vec::new();

    // Handle param (receiver)
    let has_receiver = method.receiver != MetaReceiver::None;

    if has_receiver {
        // All handles are heap-allocated, passed as *mut c_void.
        // Builder by-value methods take *mut *mut c_void (pointer to the
        // caller's handle variable) so the bridge can swap the pointer.
        params.push(CExternParam {
            name: format_ident!("handle"),
            c_type: quote! { *mut core::ffi::c_void },
        });
    }

    // Regular params
    for p in &method.params {
        if matches!(p.kind, MetaParamKind::StrSlice) {
            params.push(CExternParam {
                name: p.name.clone(),
                c_type: c_param_type(&p.kind, None, &ctx),
            });
            params.push(CExternParam {
                name: format_ident!("{}_len", p.name),
                c_type: quote! { usize },
            });
        } else {
            params.push(CExternParam {
                name: p.name.clone(),
                c_type: c_param_type(&p.kind, Some(p.rust_type()), &ctx),
            });
        }
    }

    // Return type + out-param for handle returns or Result
    let ret = match &method.ret {
        MetaReturn::Void => quote! {},
        MetaReturn::Value(_vk) => {
            // All values (handles and primitives) returned directly.
            // Handles return *mut c_void, primitives return their CRepr.
            let ty = c_return_type(_vk, &method.rust_ret, &ctx);
            quote! { -> #ty }
        }
        MetaReturn::Result { ok, .. } => {
            let ok_is_handle = ok.is_some()
                && is_result_ok_handle(&method.rust_ret, handle_types);

            // Builder by-value Result<Self, E>: the ok value is written back
            // through the double-pointer handle param, not returned. Use
            // FtResult style, not GLib-style.
            let is_builder_self_result = method.is_builder()
                && method.receiver == MetaReceiver::Value;

            if ok_is_handle && !is_builder_self_result {
                // GLib-style: return handle directly (NULL on error).
                // err_out is *mut *mut c_void (pointer to caller's FtError variable).
                params.push(CExternParam {
                    name: format_ident!("err_out"),
                    c_type: quote! { *mut *mut core::ffi::c_void },
                });
                quote! { -> *mut core::ffi::c_void }
            } else {
                // FtResult style: return packed error code.
                // Builder-self-result writes ok back through the handle
                // double-pointer, so no separate result out-param.
                if !is_builder_self_result {
                    if let Some(vk) = ok {
                        let ok_rust_type = extract_result_ok_type(&method.rust_ret);
                        params.push(CExternParam {
                            name: format_ident!("result"),
                            c_type: c_out_param_type(vk, &ok_rust_type, &ctx),
                        });
                    }
                }
                // err_out is *mut *mut c_void (pointer to caller's FtError variable).
                params.push(CExternParam {
                    name: format_ident!("err_out"),
                    c_type: quote! { *mut *mut core::ffi::c_void },
                });
                quote! { -> ffier::FfierResult }
            }
        }
    };

    CExternSignature {
        fn_name,
        params,
        ret,
    }
}

/// Produce the concrete C type tokens for a parameter kind.
///
/// This is the canonical "what C type does this parameter have?" function.
/// Both the bridge generator and the client generator use it to ensure
/// their extern declarations agree.
///
/// Produce the C type tokens for a parameter.
pub fn c_param_type(
    kind: &MetaParamKind,
    rust_type: Option<&TokenStream2>,
    ctx: &SignatureContext,
) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular(MetaTypePair { bridge_type, .. }) => {
            let ty = match ctx {
                SignatureContext::Client => {
                    let rt = rust_type.expect("Regular param must have rust_type");
                    erase_lifetimes_tokens(rt)
                }
                SignatureContext::Bridge => bridge_type.clone(),
            };
            quote! { <#ty as ffier::FfiType>::CRepr }
        }
        MetaParamKind::ImplTrait { .. } => quote! { *mut core::ffi::c_void },
        MetaParamKind::StrSlice => quote! { *const ffier::FfierBytes },
    }
}

/// Produce the C return type tokens for a value kind.
pub fn c_return_type(
    kind: &MetaTypePair,
    rust_ret: &TokenStream2,
    ctx: &SignatureContext,
) -> TokenStream2 {
    let bridge_type = &kind.bridge_type;
    let ty = match ctx {
        SignatureContext::Client => erase_lifetimes_tokens(rust_ret),
        SignatureContext::Bridge => bridge_type.clone(),
    };
    quote! { <#ty as ffier::FfiType>::CRepr }
}

/// Produce the C type for a Result ok-value out-parameter.
pub fn c_out_param_type(
    kind: &MetaTypePair,
    rust_ret: &TokenStream2,
    ctx: &SignatureContext,
) -> TokenStream2 {
    let inner = c_return_type(kind, rust_ret, ctx);
    quote! { *mut #inner }
}

fn meta_param_conversion(
    id: &syn::Ident,
    kind: &MetaParamKind,
    len_ident: Option<&syn::Ident>,
) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular(MetaTypePair { bridge_type, .. }) => {
            quote! { <#bridge_type as ffier::FfiType>::from_c(#id) }
        }
        MetaParamKind::StrSlice => {
            let len_id = len_ident.expect("StrSlice conversion needs len_ident");
            quote! { {
                let __slice = unsafe { core::slice::from_raw_parts(#id, #len_id) };
                let __strs: Vec<&str> = __slice.iter().map(|b| unsafe {
                    core::str::from_utf8_unchecked(
                        core::slice::from_raw_parts(b.data, b.len))
                }).collect();
                __strs
            } }
        }
        MetaParamKind::ImplTrait { .. } => {
            quote! { compile_error!("ImplTrait should not use param_conversion") }
        }
    }
}

fn meta_param_c_type_expr(kind: &MetaParamKind, type_pfx: &str) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular(MetaTypePair { bridge_type, .. }) => {
            quote! { &ffier_bridge::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx) }
        }
        MetaParamKind::StrSlice => {
            quote! { compile_error!("StrSlice should not use param_c_type_expr") }
        }
        MetaParamKind::ImplTrait { trait_name, .. } => {
            let full_name = format!("{type_pfx}{trait_name}");
            quote! { #full_name }
        }
    }
}

/// Format a C type name with the library prefix.
///
/// - Names starting with "Ffier" (e.g. "FfierStr") → replace prefix: "ExStr"
/// - Names starting with uppercase (e.g. "Widget") → prepend prefix: "ExWidget"
/// - Everything else (e.g. "int32_t", "bool") → use as-is
pub fn format_c_type_name(c_type_name: &str, type_pfx: &str) -> String {
    if let Some(suffix) = c_type_name.strip_prefix("Ffier") {
        format!("{type_pfx}{suffix}")
    } else if c_type_name.starts_with(|c: char| c.is_uppercase()) {
        format!("{type_pfx}{c_type_name}")
    } else {
        c_type_name.to_string()
    }
}

// ---------------------------------------------------------------------------
// Header line + doxygen helpers (ported from lib.rs for standalone use)
// ---------------------------------------------------------------------------

fn build_header_line(
    c_ret_expr: TokenStream2,
    ffi_name_str: &str,
    handle_type: Option<&String>,
    param_c_type_exprs: &[TokenStream2],
    param_name_strs: &[&String],
    out_param_c_type: Option<&TokenStream2>,
    // If true, append `FtError* err_out` as the last parameter.
    has_err_out: bool,
    // C type name for error handle (e.g. "FtError"), used only when has_err_out.
    err_handle_c_name: Option<&str>,
) -> TokenStream2 {
    let out_snippet = out_param_c_type.map(|ct| {
        quote! {
            if need_comma { s.push_str(", "); }
            s.push_str(#ct);
            s.push_str("* result");
            need_comma = true;
        }
    });
    let err_out_snippet = if has_err_out {
        let err_c = err_handle_c_name.unwrap_or("void*");
        Some(quote! {
            if need_comma { s.push_str(", "); }
            s.push_str(#err_c);
            s.push_str("* err_out");
            need_comma = true;
        })
    } else {
        None
    };
    let handle_snippet = handle_type.map(|ht| {
        quote! {
            s.push_str(#ht);
            need_comma = true;
        }
    });
    quote! {{
        let c_type_names: &[&str] = &[#(#param_c_type_exprs),*];
        let param_names: &[&str] = &[#(#param_name_strs),*];
        let mut s = String::new();
        s.push_str(#c_ret_expr);
        s.push(' ');
        s.push_str(#ffi_name_str);
        s.push('(');
        let mut need_comma = false;
        #handle_snippet
        for (cty, name) in c_type_names.iter().zip(param_names.iter()) {
            if need_comma { s.push_str(", "); }
            s.push_str(cty);
            s.push(' ');
            s.push_str(name);
            need_comma = true;
        }
        #out_snippet
        #err_out_snippet
        if !need_comma { s.push_str("void"); }
        s.push_str(");");
        s
    }}
}

/// Parsed doc comment sections.
struct DocSections {
    body: Vec<String>,
    param_docs: Vec<(String, String)>,
    returns_doc: Option<String>,
}

fn parse_doc_sections(doc_lines: &[String]) -> DocSections {
    let mut body = Vec::new();
    let mut param_docs = Vec::new();
    let mut returns_doc = None;

    enum Section {
        Body,
        Arguments,
        Returns,
    }
    let mut section = Section::Body;
    let mut returns_lines: Vec<String> = Vec::new();

    for raw in doc_lines {
        let line = raw.strip_prefix(' ').unwrap_or(raw);

        let lower = line.trim().to_lowercase();
        if lower == "# arguments" || lower == "# parameters" {
            section = Section::Arguments;
            continue;
        }
        if lower.starts_with("# return") {
            section = Section::Returns;
            continue;
        }
        if line.trim().starts_with("# ") {
            if !returns_lines.is_empty() {
                returns_doc = Some(returns_lines.join(" ").trim().to_string());
                returns_lines.clear();
            }
            section = Section::Body;
        }

        match section {
            Section::Body => body.push(raw.clone()),
            Section::Arguments => {
                let trimmed = line.trim();
                let after_bullet = trimmed
                    .strip_prefix("* ")
                    .or_else(|| trimmed.strip_prefix("- "));
                if let Some(rest) = after_bullet
                    && let Some((name, desc)) = parse_param_entry(rest)
                {
                    param_docs.push((name, desc));
                }
            }
            Section::Returns => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    returns_lines.push(trimmed.to_string());
                }
            }
        }
    }

    if !returns_lines.is_empty() {
        returns_doc = Some(returns_lines.join(" ").trim().to_string());
    }

    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }

    DocSections {
        body,
        param_docs,
        returns_doc,
    }
}

fn parse_param_entry(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let rest = s.strip_prefix('`')?;
    let end = rest.find('`')?;
    let name = rest[..end].to_string();
    let after = rest[end + 1..].trim();
    // Strip separator between name and description: `-`, `:`, or `—`
    let desc = after
        .strip_prefix('-')
        .or_else(|| after.strip_prefix(':'))
        .or_else(|| after.strip_prefix('\u{2014}'))
        .unwrap_or(after)
        .trim()
        .to_string();
    Some((name, desc))
}

fn build_doxygen_comment(
    doc_lines: &[String],
    param_names: &[String],
    has_out_param: bool,
    err_c_name: Option<&str>,
) -> Option<String> {
    if doc_lines.is_empty() {
        return None;
    }

    let sections = parse_doc_sections(doc_lines);

    if sections.body.is_empty() && sections.param_docs.is_empty() && sections.returns_doc.is_none()
    {
        return None;
    }

    let mut out = String::from("/**\n");

    for line in &sections.body {
        let trimmed = line.strip_prefix(' ').unwrap_or(line);
        if trimmed.is_empty() {
            out.push_str(" *\n");
        } else {
            out.push_str(&format!(" * {trimmed}\n"));
        }
    }

    let has_params = !param_names.is_empty() || has_out_param;
    let has_return = err_c_name.is_some() || sections.returns_doc.is_some();
    if has_params || has_return {
        out.push_str(" *\n");
    }

    for name in param_names {
        let desc = sections
            .param_docs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d.as_str())
            .unwrap_or("");
        if desc.is_empty() {
            out.push_str(&format!(" * @param {name}\n"));
        } else {
            out.push_str(&format!(" * @param {name} {desc}\n"));
        }
    }

    if has_out_param {
        if let Some(ref doc) = sections.returns_doc {
            out.push_str(&format!(" * @param[out] result {doc}\n"));
        } else {
            out.push_str(" * @param[out] result\n");
        }
    }

    if let Some(err_name) = err_c_name {
        out.push_str(&format!(
            " * @return {err_name} with code 0 on success, error code on failure.\n"
        ));
    } else if let Some(ref doc) = sections.returns_doc {
        out.push_str(&format!(" * @return {doc}\n"));
    }

    out.push_str(" */");
    Some(out)
}

// ===========================================================================
// HeaderSection / HeaderBuilder --- structured C header generation
// ===========================================================================

pub struct HeaderSection {
    pub struct_name: String,
    pub handle_typedef: String,
    pub declarations: String,
}

pub struct HeaderBuilder {
    guard: String,
    shared_types: String,
    sections: Vec<HeaderSection>,
}

// TODO: HeaderBuilder should accept the prefix (e.g. "krun") instead of a raw guard string,
// and derive the header guard, handle typedefs, shared type names, and macro names from it.
// The prefix should only be specified here, not duplicated in each #[ffier::exportable(prefix = "krun")].

impl HeaderBuilder {
    pub fn new(guard: &str, shared_types: String) -> Self {
        Self {
            guard: guard.to_string(),
            shared_types,
            sections: Vec::new(),
        }
    }

    pub fn push(mut self, section: HeaderSection) -> Self {
        self.sections.push(section);
        self
    }

    pub fn build(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!("#ifndef {}\n", self.guard));
        out.push_str(&format!("#define {}\n", self.guard));
        out.push('\n');
        out.push_str("#include <stddef.h>\n");
        out.push_str("#include <stdint.h>\n");
        out.push_str("#include <stdbool.h>\n");
        out.push_str("#include <string.h>\n");
        out.push('\n');

        // Collect all handle typedefs
        let mut has_handle = false;
        for section in &self.sections {
            if !section.handle_typedef.is_empty() {
                out.push_str(&section.handle_typedef);
                out.push('\n');
                has_handle = true;
            }
        }
        if has_handle {
            out.push('\n');
        }

        // Emit shared types (Str/Bytes/Path structs + macros)
        if !self.shared_types.is_empty() {
            out.push_str(&self.shared_types);
            out.push('\n');
        }

        out.push_str("/* Header auto-generated by ffier */\n");

        // Per-section declarations
        for section in &self.sections {
            if !section.declarations.is_empty() {
                // Ensure blank line before section comment
                if !out.ends_with("\n\n") {
                    if out.ends_with('\n') {
                        out.push('\n');
                    } else {
                        out.push_str("\n\n");
                    }
                }
                let name = &section.struct_name;
                let dashes = "-".repeat(72 - 6 - name.len()); // 72 cols total
                out.push_str(&format!("/* {name} {dashes} */\n\n"));
                out.push_str(&section.declarations);
            }
        }

        out.push('\n');
        out.push_str(&format!("#endif /* {} */\n", self.guard));
        out
    }
}

// ===========================================================================
// Self-dispatch bridge generation
// ===========================================================================

/// Emit a `let obj = ...` binding that borrows a value from a handle.
///
/// - `mutable = false`: `let obj = ffier::ffier_handle_borrow::<T>(handle);`
/// - `mutable = true`:  `let obj = ffier::ffier_handle_borrow_mut::<T>(handle);`
fn borrow_from_handle(ty: &TokenStream2, mutable: bool) -> TokenStream2 {
    if mutable {
        quote! {
            let obj = unsafe { ffier::ffier_handle_borrow_mut::<#ty>(handle) };
        }
    } else {
        quote! {
            let obj = unsafe { ffier::ffier_handle_borrow::<#ty>(handle) };
        }
    }
}

/// Generate per-trait dispatching C functions for an `#[ffier::implementable]` trait.
///
/// For each method on the trait, generates a single C function that reads the
/// type tag from the handle and dispatches to the concrete implementor's method.
/// Also generates a dispatching destroy function.
///
/// Example: for `trait Fruit` with method `value(&self) -> i32` and
/// implementors `Apple`, `Orange`, `VtableFruit`:
///
/// ```c
/// int32_t ft_fruit_value(void* handle);
/// void ft_fruit_destroy(void* handle);
/// ```
fn generate_self_dispatch_bridge(
    trait_name: &str,
    info: &TraitDispatchInfo,
    prefix: &str,
    trait_map: &TraitMap,
) -> TokenStream2 {
    let imp = info
        .implementable
        .as_ref()
        .expect("generate_self_dispatch_bridge called for non-implementable trait");
    let trait_path = &imp.trait_path;
    let trait_snake = camel_to_snake(trait_name);
    let fn_pfx = format!("{prefix}_");
    let type_pfx = ffier_meta::snake_to_pascal(prefix);

    let section_name = format!("{trait_name} (dispatch)");
    let header_fn_name = format_ident!("{fn_pfx}{trait_snake}__dispatch_header");
    let accepted_const = format_ident!("__FFIER_ACCEPTED_{trait_name}");

    let mut bridge_fns = Vec::new();
    let mut header_lines: Vec<TokenStream2> = Vec::new();

    // Generate dispatching functions for each trait method (own methods only,
    // not supertrait methods — those need their own dispatch via their own trait).
    // TODO: Reconsider the `supers(...)` syntax on #[ffier::implementable].
    //       Perhaps supertrait methods should be exported by making the supertrait
    //       itself #[ffier::implementable] (or at least having its own trait_impl
    //       entries), rather than inlining them into the subtrait's vtable.
    let own_methods = &imp.methods[..imp.own_method_count];
    for m in own_methods.iter() {
        let method_name = &m.name;
        let ffi_name_str = format!("{fn_pfx}{trait_snake}_{method_name}");
        let ffi_name = format_ident!("{ffi_name_str}");

        // Build C bridge params: handle + method params
        let mut bridge_params = vec![quote! { handle: *mut core::ffi::c_void }];
        let mut call_args = Vec::new();
        // Pre-dispatch borrow statements for impl Trait params (resolved
        // from TraitMap at codegen time, borrowing the handle as the
        // concrete VtableWrapper type).
        let mut impl_trait_borrows = Vec::new();

        for p in &m.params {
            let param_name = &p.name;
            if let Some(trait_name) = p.impl_trait_name() {
                // impl Trait param — the C param is a raw handle pointer.
                bridge_params.push(quote! { #param_name: *mut core::ffi::c_void });

                // Resolve the wrapper type from TraitMap.
                let wrapper_path = trait_map
                    .get(trait_name)
                    .and_then(|info| info.implementable.as_ref())
                    .map(|imp| &imp.wrapper_path)
                    .unwrap_or_else(|| panic!(
                        "impl Trait param `{}` references trait `{}` which has no #[implementable] entry in the library",
                        param_name, trait_name,
                    ));

                let borrow_name = format_ident!("__impl_trait_{param_name}");
                impl_trait_borrows.push(quote! {
                    let #borrow_name = unsafe {
                        ffier::ffier_handle_borrow_mut::<#wrapper_path>(#param_name)
                    };
                });
                call_args.push(quote! { #borrow_name });
            } else {
                let bt = p.bridge_type();
                bridge_params.push(quote! { #param_name: <#bt as ffier::FfiType>::CRepr });
                call_args.push(quote! { <#bt as ffier::FfiType>::from_c(#param_name) });
            }
        }

        // Return type
        let ret_type = match &m.ret {
            MetaReturn::Void => quote! {},
            MetaReturn::Value(MetaTypePair { bridge_type, .. }) => {
                quote! { -> <#bridge_type as ffier::FfiType>::CRepr }
            }
            MetaReturn::Result { .. } => unreachable!("Result returns not yet supported in trait methods"),
        };

        // Build dispatch branches — one per variant.
        // TODO: Support `&mut self` and consuming `self` receivers once
        //       MetaVtableMethod tracks receiver kind.
        let wrapper_path = &imp.wrapper_path;
        let method_index_u32 = m.index() as u32;

        // For defaulted methods, the VtableFoo dispatch branch needs a
        // metadata check: if the handle's metadata field has bit 0 set
        // and the method index matches, skip vtable dispatch and call
        // the library's default directly. This is used by client-side
        // trait defaults to prevent infinite re-entrancy.
        // Build the full path to the default helper function.
        // The helper is generated by #[ffier::implementable] next to the trait
        // definition, so its path is: trait_path's parent module :: __ffier_default_TraitName_method
        let default_helper_path = if m.has_default() {
            let helper_ident = format_ident!("__ffier_default_{}_{}", trait_name, method_name);
            // Replace the last segment of trait_path with the helper name
            let mut tokens: Vec<proc_macro2::TokenTree> = trait_path.clone().into_iter().collect();
            if let Some(last) = tokens.last_mut() {
                *last = proc_macro2::TokenTree::Ident(helper_ident);
            }
            Some(tokens.into_iter().collect::<TokenStream2>())
        } else {
            None
        };

        let dispatch_branches: Vec<_> = info.variants.iter().map(|v| {
            let ty = &v.bridge_type;
            let is_vtable_variant = ty.to_string() == wrapper_path.to_string();

            // For the VtableFoo variant of defaulted methods, check metadata
            // before calling through the vtable.
            let metadata_guard = if is_vtable_variant && m.has_default() {
                if let Some(helper) = &default_helper_path {
                    let obj_for_default = borrow_from_handle(ty, m.is_mut());
                    let default_call = match &m.ret {
                        MetaReturn::Void => quote! {
                            #helper(obj #(, #call_args)*);
                            return;
                        },
                        MetaReturn::Value(MetaTypePair { bridge_type, .. }) => quote! {
                            let call_result = #helper(obj #(, #call_args)*);
                            return <#bridge_type as ffier::FfiType>::into_c(call_result);
                        },
                        MetaReturn::Result { .. } => unreachable!("Result returns not yet supported in trait methods"),
                    };
                    quote! {
                        let __metadata = unsafe { ffier::handle_metadata(handle) };
                        if __metadata & 2 != 0 && (__metadata >> 2) & 0x7FFF == #method_index_u32 {
                            #obj_for_default
                            #default_call
                        }
                    }
                } else {
                    quote! {}
                }
            } else {
                quote! {}
            };

            if m.raw_handle() {
                // raw_handle: cast handle to *const FfierHandle<T> and pass directly
                let ret_conversion = match &m.ret {
                    MetaReturn::Void => quote! {
                        <#ty as #trait_path>::#method_name(
                            handle as *const ffier::FfierHandle<#ty> #(, #call_args)*
                        );
                    },
                    MetaReturn::Value(MetaTypePair { bridge_type, .. }) => quote! {
                        let call_result = <#ty as #trait_path>::#method_name(
                            handle as *const ffier::FfierHandle<#ty> #(, #call_args)*
                        );
                        return <#bridge_type as ffier::FfiType>::into_c(call_result);
                    },
                    MetaReturn::Result { .. } => unreachable!("Result returns not yet supported in trait methods"),
                };
                quote! {
                    if __type_tag == <#ty as ffier::FfiHandle>::TYPE_TAG {
                        #ret_conversion
                    }
                }
            } else {
                let obj_binding = borrow_from_handle(ty, m.is_mut());
                let ret_conversion = match &m.ret {
                    MetaReturn::Void => quote! {
                        <#ty as #trait_path>::#method_name(obj #(, #call_args)*);
                    },
                    MetaReturn::Value(MetaTypePair { bridge_type, .. }) => quote! {
                        let call_result = <#ty as #trait_path>::#method_name(obj #(, #call_args)*);
                        return <#bridge_type as ffier::FfiType>::into_c(call_result);
                    },
                    MetaReturn::Result { .. } => unreachable!("Result returns not yet supported in trait methods"),
                };
                quote! {
                    if __type_tag == <#ty as ffier::FfiHandle>::TYPE_TAG {
                        #metadata_guard
                        #obj_binding
                        #ret_conversion
                    }
                }
            }
        }).collect();

        let expected_str = format!("{trait_name} implementor");
        bridge_fns.push(quote! {
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn #ffi_name(#(#bridge_params),*) #ret_type {
                #(#impl_trait_borrows)*
                let __type_tag = unsafe { ffier::handle_type_tag(handle) };
                #(#dispatch_branches else)* {
                    __ffier_dispatch_panic(#ffi_name_str, #expected_str, #accepted_const, __type_tag);
                }
            }
        });

        // Header line for this method
        let (param_id_strs, param_type_exprs) = vtable_param_c_types(&m.params, &type_pfx);
        let ret_c_expr = vtable_ret_c_expr(&m.ret, &type_pfx);

        header_lines.push(quote! {{
            let mut s = String::new();
            s.push_str(#ret_c_expr);
            s.push(' ');
            s.push_str(#ffi_name_str);
            s.push_str("(void* handle");
            let param_types: &[&str] = &[#(#param_type_exprs),*];
            let param_names: &[&str] = &[#(#param_id_strs),*];
            for (t, n) in param_types.iter().zip(param_names.iter()) {
                s.push_str(", ");
                s.push_str(t);
                s.push(' ');
                s.push_str(n);
            }
            s.push_str(");");
            s
        }});
    }

    // Generate dispatching destroy function
    let destroy_name_str = format!("{fn_pfx}{trait_snake}_destroy");
    let destroy_name = format_ident!("{destroy_name_str}");

    let destroy_branches: Vec<_> = info
        .variants
        .iter()
        .map(|v| {
            let ty = &v.bridge_type;
            quote! {
                if __type_tag == <#ty as ffier::FfiHandle>::TYPE_TAG {
                    unsafe { ffier::ffier_handle_drop::<#ty>(handle) };
                }
            }
        })
        .collect();

    let destroy_expected = format!("{trait_name} implementor");
    bridge_fns.push(quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #destroy_name(handle: *mut core::ffi::c_void) {
            if !handle.is_null() {
                let __type_tag = unsafe { ffier::handle_type_tag(handle) };
                #(#destroy_branches else)* {
                    __ffier_dispatch_panic(#destroy_name_str, #destroy_expected, #accepted_const, __type_tag);
                }
            }
        }
    });

    header_lines.push(quote! {
        concat!("void ", #destroy_name_str, "(void* handle);")
    });

    let num_header_lines = header_lines.len();

    quote! {
        #(#bridge_fns)*

        pub fn #header_fn_name() -> ffier_bridge::HeaderSection {
            let decl_lines: [String; #num_header_lines] = [
                #(#header_lines .to_string()),*
            ];
            ffier_bridge::HeaderSection {
                struct_name: #section_name.to_string(),
                handle_typedef: String::new(),
                declarations: decl_lines.join("\n"),
            }
        }
    }
}

// ===========================================================================
// Trait impl bridge generation
// ===========================================================================

/// Extract C type expressions and param names from vtable method params.
fn vtable_param_c_types(
    params: &[ffier_meta::MetaParam],
    type_pfx: &str,
) -> (Vec<String>, Vec<TokenStream2>) {
    let mut names = Vec::new();
    let mut types = Vec::new();
    for p in params {
        names.push(p.name.to_string());
        if p.is_impl_trait() {
            // impl Trait params are raw handle pointers in C.
            types.push(quote! { "void*" });
        } else {
            let bt = p.bridge_type();
            types.push(quote! {
                &ffier_bridge::format_c_type_name(<#bt as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
            });
        }
    }
    (names, types)
}

/// C return type expression for a vtable return.
fn vtable_ret_c_expr(ret: &MetaReturn, type_pfx: &str) -> TokenStream2 {
    match ret {
        MetaReturn::Void => quote! { "void" },
        MetaReturn::Value(MetaTypePair { bridge_type, .. }) => quote! {
            &ffier_bridge::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
        },
        MetaReturn::Result { .. } => unreachable!("Result returns not yet supported in trait methods"),
    }
}

fn generate_trait_impl_bridge(meta: MetaTraitImpl, trait_map: &TraitMap) -> TokenStream2 {
    let struct_path = &meta.struct_path;
    let trait_path = &meta.trait_path;
    let fn_pfx = meta.fn_pfx();
    let type_pfx = meta.type_pfx();
    let struct_name_str = meta.struct_name.to_string();
    let struct_snake = camel_to_snake(&struct_name_str);
    let trait_name_str = meta.trait_name.to_string();
    let trait_snake = camel_to_snake(&trait_name_str);

    let header_fn_name = format_ident!("{fn_pfx}{trait_snake}_for_{struct_snake}__header");
    let section_name = format!("{trait_name_str} for {struct_name_str}");

    let mut bridge_fns = Vec::new();
    let mut header_lines: Vec<TokenStream2> = Vec::new();

    for m in &meta.methods {
        let method_name = &m.name;
        let ffi_name_str = format!("{fn_pfx}{struct_snake}_{method_name}");
        let ffi_name = format_ident!("{ffi_name_str}");

        // C params for the bridge function
        let mut bridge_params = vec![quote! { handle: *mut core::ffi::c_void }];
        let mut call_args = Vec::new();
        let mut impl_trait_borrows = Vec::new();

        for p in &m.params {
            let param_name = &p.name;
            if let Some(impl_trait_name) = p.impl_trait_name() {
                bridge_params.push(quote! { #param_name: *mut core::ffi::c_void });
                let wrapper_path = trait_map
                    .get(impl_trait_name)
                    .and_then(|info| info.implementable.as_ref())
                    .map(|imp| &imp.wrapper_path)
                    .unwrap_or_else(|| panic!(
                        "impl Trait param `{}` references trait `{}` which has no #[implementable] entry",
                        param_name, impl_trait_name,
                    ));
                let borrow_name = format_ident!("__impl_trait_{param_name}");
                impl_trait_borrows.push(quote! {
                    let #borrow_name = unsafe {
                        ffier::ffier_handle_borrow_mut::<#wrapper_path>(#param_name)
                    };
                });
                call_args.push(quote! { #borrow_name });
            } else {
                let bt = p.bridge_type();
                bridge_params.push(quote! { #param_name: <#bt as ffier::FfiType>::CRepr });
                call_args.push(quote! { <#bt as ffier::FfiType>::from_c(#param_name) });
            }
        }

        // Return type
        let (ret_type, ret_conversion) = match &m.ret {
            MetaReturn::Void => (quote! {}, quote! { call_result }),
            MetaReturn::Value(MetaTypePair { bridge_type, .. }) => (
                quote! { -> <#bridge_type as ffier::FfiType>::CRepr },
                quote! { <#bridge_type as ffier::FfiType>::into_c(call_result) },
            ),
            MetaReturn::Result { .. } => unreachable!("Result returns not yet supported in trait methods"),
        };

        let fn_body = if m.raw_handle() {
            // raw_handle: cast handle and pass directly, no &self borrow
            quote! {
                let call_result = <#struct_path as #trait_path>::#method_name(
                    handle as *const ffier::FfierHandle<#struct_path> #(, #call_args)*
                );
                #ret_conversion
            }
        } else {
            let borrow = borrow_from_handle(&quote! { #struct_path }, m.is_mut());
            quote! {
                #borrow
                let call_result = <#struct_path as #trait_path>::#method_name(obj, #(#call_args),*);
                #ret_conversion
            }
        };

        bridge_fns.push(quote! {
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn #ffi_name(#(#bridge_params),*) #ret_type {
                #(#impl_trait_borrows)*
                #fn_body
            }
        });

        // Header line
        let (param_id_strs, param_type_exprs) = vtable_param_c_types(&m.params, &type_pfx);
        let ret_c_expr = vtable_ret_c_expr(&m.ret, &type_pfx);
        let handle_c_name = format!("{type_pfx}{struct_name_str}");

        header_lines.push(quote! {{
            let mut s = String::new();
            s.push_str(#ret_c_expr);
            s.push(' ');
            s.push_str(#ffi_name_str);
            s.push('(');
            s.push_str(#handle_c_name);
            s.push_str(" handle");
            let param_types: &[&str] = &[#(#param_type_exprs),*];
            let param_names: &[&str] = &[#(#param_id_strs),*];
            for (t, n) in param_types.iter().zip(param_names.iter()) {
                s.push_str(", ");
                s.push_str(t);
                s.push(' ');
                s.push_str(n);
            }
            s.push_str(");");
            s
        }});
    }

    let num_header_lines = header_lines.len();

    quote! {
        #(#bridge_fns)*

        pub fn #header_fn_name() -> ffier_bridge::HeaderSection {
            let decl_lines: [String; #num_header_lines] = [
                #(#header_lines .to_string()),*
            ];
            ffier_bridge::HeaderSection {
                struct_name: #section_name.to_string(),
                handle_typedef: String::new(),
                declarations: decl_lines.join("\n"),
            }
        }
    }
}

// ===========================================================================
// JSON metadata emission
// ===========================================================================

/// Convert parsed metadata to `ffier_schema::Library` and write to
/// `target/ffier-{prefix}.json` relative to the workspace root.
fn emit_json(
    prefix: &str,
    errors: &[TokenStream2],
    exportables: &[TokenStream2],
    implementables: &[TokenStream2],
    trait_impls: &[TokenStream2],
) {
    // CARGO_MANIFEST_DIR is always set by cargo when rustc runs, even without
    // a build.rs. We walk up to find the workspace target/ directory.
    let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(d) => d,
        Err(_) => return,
    };
    let target_dir = match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) => std::path::PathBuf::from(d),
        Err(_) => {
            // Walk up from manifest dir to find target/
            let mut dir = std::path::PathBuf::from(&manifest_dir);
            loop {
                let candidate = dir.join("target");
                if candidate.is_dir() {
                    break candidate;
                }
                if !dir.pop() {
                    return; // can't find target dir
                }
            }
        }
    };

    let library = build_schema(prefix, errors, exportables, implementables, trait_impls);
    let json = library.to_json();
    let path = target_dir.join(format!("ffier-{prefix}.json"));
    std::fs::write(&path, json).unwrap_or_else(|e| {
        panic!("failed to write {}: {e}", path.display());
    });
}

/// Context for C type resolution during schema conversion.
struct CTypeResolver {
    type_pfx: String,       // e.g. "Ft"
    upper_pfx: String,      // e.g. "FT_"
    fn_pfx: String,         // e.g. "ft_"
}

impl CTypeResolver {
    fn new(prefix: &str) -> Self {
        let type_pfx = ffier_meta::snake_to_pascal(prefix);
        let upper_pfx = format!("{}_", prefix.to_ascii_uppercase());
        let fn_pfx = format!("{prefix}_");
        CTypeResolver { type_pfx, upper_pfx, fn_pfx }
    }

    /// Resolve the C type for a handle (always opaque pointer typedef).
    fn handle_c_name(&self, name: &str) -> String {
        format!("{}{}", self.type_pfx, name)
    }

    /// FFI function name: prefix + ffi_name suffix.
    fn ffi_fn_name(&self, ffi_name: &str) -> String {
        format!("{}{}", self.fn_pfx, ffi_name)
    }

    /// Error constant name: FT_ERROR_TEST_NOT_FOUND
    /// Strips "Error" suffix from the error type name: TestError → TEST.
    fn error_const_name(&self, error_name: &str, variant_name: &str) -> String {
        let stripped = error_name.strip_suffix("Error").unwrap_or(error_name);
        let error_upper = camel_to_upper_snake(stripped);
        let variant_upper = camel_to_upper_snake(variant_name);
        format!("{}ERROR_{}_{}", self.upper_pfx, error_upper, variant_upper)
    }

    /// Parse a Rust type token string (e.g. `"& 'a Widget"`, `"View < 'a >"`,
    /// `"i32"`) into a `TypeRef`.
    fn to_type_ref(&self, rust_type: &str) -> ffier_schema::TypeRef {
        let s = rust_type.trim();

        // Parse reference: & or &mut, with optional lifetime.
        // TokenStream renders both `&mut 'a T` and `&'a mut T` forms.
        let (ref_kind, ref_lifetime, after_ref) = if let Some(rest) = s.strip_prefix('&') {
            let rest = rest.trim();
            // Check for `mut` before lifetime: `& mut 'a T`
            let (mut is_mut, rest) = if let Some(r) = rest.strip_prefix("mut ") {
                (true, r.trim())
            } else {
                (false, rest)
            };
            // Check for lifetime: `'a`
            let (lifetime, rest) = if rest.starts_with('\'') {
                let after_tick = &rest[1..];
                let lt_len = after_tick
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(after_tick.len());
                let lt = &rest[1..1 + lt_len];
                (Some(lt.to_string()), rest[1 + lt_len..].trim())
            } else {
                (None, rest)
            };
            // Check for `mut` after lifetime: `& 'a mut T`
            let rest = if !is_mut {
                if let Some(r) = rest.strip_prefix("mut ") {
                    is_mut = true;
                    r.trim()
                } else {
                    rest
                }
            } else {
                rest
            };
            let rk = if is_mut { ffier_schema::RefKind::Mut } else { ffier_schema::RefKind::Shared };
            (rk, lifetime, rest)
        } else {
            (ffier_schema::RefKind::None, None, s)
        };

        // Parse type name and generic lifetime args: `View < 'a >` → name="View", args=["a"]
        let (type_name, type_args) = if let Some(angle_pos) = after_ref.find('<') {
            let name = after_ref[..angle_pos].trim();
            let args_str = &after_ref[angle_pos + 1..];
            let args_str = args_str.trim().trim_end_matches('>').trim();
            let type_args: Vec<String> = args_str
                .split(',')
                .map(|a| a.trim().trim_start_matches('\'').to_string())
                .filter(|a| !a.is_empty())
                .collect();
            (name, type_args)
        } else {
            (after_ref, vec![])
        };

        ffier_schema::TypeRef {
            type_name: type_name.to_string(),
            ref_kind,
            ref_lifetime,
            type_args,
        }
    }
}

fn build_schema(
    prefix: &str,
    errors: &[TokenStream2],
    exportables: &[TokenStream2],
    implementables: &[TokenStream2],
    trait_impls: &[TokenStream2],
) -> ffier_schema::Library {
    let errors_parsed: Vec<_> = errors
        .iter()
        .filter_map(|item| syn::parse2::<MetaError>(item.clone()).ok())
        .collect();
    let exportables_parsed: Vec<_> = exportables
        .iter()
        .filter_map(|item| syn::parse2::<MetaExportable>(item.clone()).ok())
        .collect();
    let implementables_parsed: Vec<_> = implementables
        .iter()
        .filter_map(|item| syn::parse2::<MetaImplementable>(item.clone()).ok())
        .collect();
    let trait_impls_parsed: Vec<_> = trait_impls
        .iter()
        .filter_map(|item| syn::parse2::<MetaTraitImpl>(item.clone()).ok())
        .collect();

    let resolver = CTypeResolver::new(prefix);

    // Build type registry
    let mut type_registry = std::collections::BTreeMap::new();

    // Primitives
    for (name, c_type) in &[
        ("i8", "int8_t"), ("i16", "int16_t"), ("i32", "int32_t"), ("i64", "int64_t"),
        ("u8", "uint8_t"), ("u16", "uint16_t"), ("u32", "uint32_t"), ("u64", "uint64_t"),
        ("f32", "float"), ("f64", "double"),
        ("isize", "intptr_t"), ("usize", "uintptr_t"), ("bool", "bool"),
    ] {
        type_registry.insert(name.to_string(), ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Primitive,
            c_type: c_type.to_string(),
            type_tag: None,
            lifetime_params: vec![],
        });
    }

    // Builtins
    type_registry.insert("str".to_string(), ffier_schema::TypeEntry {
        kind: ffier_schema::TypeKind::String,
        c_type: format!("{}Str", resolver.type_pfx),
        type_tag: None,
        lifetime_params: vec![],
    });
    type_registry.insert("[u8]".to_string(), ffier_schema::TypeEntry {
        kind: ffier_schema::TypeKind::Bytes,
        c_type: format!("{}Bytes", resolver.type_pfx),
        type_tag: None,
        lifetime_params: vec![],
    });

    // Std type aliases
    type_registry.insert("BorrowedFd".to_string(), ffier_schema::TypeEntry {
        kind: ffier_schema::TypeKind::Alias { alias_of: "i32".to_string(), owned: false },
        c_type: "int".to_string(),
        type_tag: None,
        lifetime_params: vec![],
    });
    type_registry.insert("OwnedFd".to_string(), ffier_schema::TypeEntry {
        kind: ffier_schema::TypeKind::Alias { alias_of: "i32".to_string(), owned: true },
        c_type: "int".to_string(),
        type_tag: None,
        lifetime_params: vec![],
    });

    // Handles (exported types)
    for e in &exportables_parsed {
        let name = e.struct_name.to_string();
        type_registry.insert(name.clone(), ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Handle,
            c_type: resolver.handle_c_name(&name),
            type_tag: Some(e.type_tag),
            lifetime_params: e.lifetimes.iter().map(|lt| lt.to_string()).collect(),
        });
    }

    // Errors
    for e in &errors_parsed {
        let name = e.name.to_string();
        type_registry.insert(name.clone(), ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Error,
            c_type: resolver.handle_c_name(&name),
            type_tag: Some(e.type_tag),
            lifetime_params: vec![],
        });
    }

    // Implementable traits
    for i in &implementables_parsed {
        let name = i.trait_name.to_string();
        type_registry.insert(name.clone(), ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Trait,
            c_type: resolver.handle_c_name(&name),
            type_tag: Some(i.type_tag),
            lifetime_params: i.trait_lifetimes.iter().map(|lt| lt.to_string()).collect(),
        });
    }

    // Traits discovered via trait_impls (no implementable annotation).
    // Infer lifetime params from the trait_lifetime_args of the impls
    // (filtering out 'static which is a concrete binding, not a param).
    for ti in &trait_impls_parsed {
        let name = ti.trait_name.to_string();
        let lifetime_params: Vec<String> = ti.trait_lifetime_args.iter()
            .filter(|lt| *lt != "static")
            .cloned()
            .collect();
        type_registry.entry(name.clone()).or_insert_with(|| ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Trait,
            c_type: resolver.handle_c_name(&name),
            type_tag: None,
            lifetime_params,
        });
    }

    ffier_schema::Library {
        prefix: prefix.to_string(),
        type_registry,
        exported_types: exportables_parsed.iter().map(|e| convert_exportable(e, &resolver)).collect(),
        errors: errors_parsed.iter().map(|e| convert_error(e, &resolver)).collect(),
        traits: implementables_parsed.iter().map(|i| convert_implementable(i, &resolver)).collect(),
        trait_impls: trait_impls_parsed.iter().map(|t| convert_trait_impl(t, &resolver)).collect(),
    }
}

fn convert_exportable(meta: &MetaExportable, r: &CTypeResolver) -> ffier_schema::ExportedType {
    let is_builder_type = meta.methods.iter().any(|m| {
        m.is_builder() && m.receiver == MetaReceiver::Value
    });
    ffier_schema::ExportedType {
        name: meta.struct_name.to_string(),
        is_builder_type,
        methods: meta.methods.iter().map(|m| convert_method(m, r)).collect(),
    }
}

fn convert_error(meta: &MetaError, r: &CTypeResolver) -> ffier_schema::ErrorType {
    ffier_schema::ErrorType {
        name: meta.name.to_string(),
        variants: meta.variants.iter().map(|v| {
            ffier_schema::ErrorVariant {
                name: v.name.to_string(),
                c_name: r.error_const_name(&meta.name.to_string(), &v.name.to_string()),
                code: v.code,
                message: v.message.clone(),
            }
        }).collect(),
    }
}

fn convert_implementable(meta: &MetaImplementable, r: &CTypeResolver) -> ffier_schema::ImplementableTrait {
    ffier_schema::ImplementableTrait {
        name: meta.trait_name.to_string(),
        methods: meta.methods.iter().map(|m| convert_method(m, r)).collect(),
        own_method_count: meta.own_method_count,
        max_vtable_slot: meta.max_vtable_slot,
    }
}

fn convert_trait_impl(meta: &MetaTraitImpl, r: &CTypeResolver) -> ffier_schema::TraitImpl {
    ffier_schema::TraitImpl {
        trait_name: meta.trait_name.to_string(),
        struct_name: meta.struct_name.to_string(),
        lifetimes: meta.lifetimes.iter().map(|lt| lt.to_string()).collect(),
        trait_lifetime_args: meta.trait_lifetime_args.clone(),
        struct_lifetime_args: meta.struct_lifetime_args.clone(),
        methods: meta.methods.iter().map(|m| convert_method(m, r)).collect(),
    }
}

fn convert_method(meta: &MetaMethod, r: &CTypeResolver) -> ffier_schema::Method {
    let context = match &meta.context {
        MetaMethodContext::Exportable { ffi_name, is_builder, .. } => {
            ffier_schema::MethodContext::Exportable {
                ffi_name: r.ffi_fn_name(ffi_name),
                is_builder: *is_builder,
            }
        }
        MetaMethodContext::Trait { has_default, index, .. } => {
            ffier_schema::MethodContext::Trait {
                index: *index,
                has_default: *has_default,
            }
        }
    };

    ffier_schema::Method {
        name: meta.name.to_string(),
        doc: meta.doc().iter().cloned().collect(),
        receiver: convert_receiver(meta.receiver),
        method_lifetimes: meta.method_lifetimes.iter().map(|lt| lt.to_string()).collect(),
        params: meta.params.iter().map(|p| convert_param(p, r)).collect(),
        ret: convert_return(&meta.ret, &meta.rust_ret, r),
        context,
    }
}

fn convert_receiver(recv: MetaReceiver) -> ffier_schema::Receiver {
    match recv {
        MetaReceiver::None => ffier_schema::Receiver::None,
        MetaReceiver::Ref => ffier_schema::Receiver::Ref,
        MetaReceiver::Mut => ffier_schema::Receiver::Mut,
        MetaReceiver::Value => ffier_schema::Receiver::Value,
    }
}

fn convert_param(p: &ffier_meta::MetaParam, r: &CTypeResolver) -> ffier_schema::Param {
    let param_type = match &p.kind {
        MetaParamKind::Regular(tp) => {
            let rt = tokens_to_string(&tp.rust_type);
            let type_ref = r.to_type_ref(&rt);
            ffier_schema::ParamType::Regular(type_ref)
        }
        MetaParamKind::StrSlice => {
            // &[&str] → two C params: pointer to FfierBytes array + length.
            // The element type is &str (a reference to str).
            let str_c = format!("{}Str", r.type_pfx);
            ffier_schema::ParamType::Slice {
                element: ffier_schema::TypeRef {
                    type_name: "str".to_string(),
                    ref_kind: ffier_schema::RefKind::Shared,
                    ref_lifetime: None,
                    type_args: vec![],
                },
                c_params: vec![
                    ffier_schema::CParam {
                        name: p.name.to_string(),
                        c_type: format!("const {str_c}*"),
                    },
                    ffier_schema::CParam {
                        name: format!("{}_len", p.name),
                        c_type: "uintptr_t".to_string(),
                    },
                ],
            }
        }
        MetaParamKind::ImplTrait { trait_name, dispatch, trait_lifetime_args, .. } => {
            ffier_schema::ParamType::ImplTrait {
                trait_name: trait_name.clone(),
                dispatch: match dispatch {
                    ffier_meta::DispatchMode::Auto => "auto".to_string(),
                    ffier_meta::DispatchMode::Concrete => "concrete".to_string(),
                    ffier_meta::DispatchMode::Vtable => "vtable".to_string(),
                },
                type_args: trait_lifetime_args.iter().map(|lt| lt.to_string()).collect(),
            }
        }
    };
    ffier_schema::Param { name: p.name.to_string(), param_type }
}

fn convert_return(
    ret: &MetaReturn,
    _rust_ret_tokens: &TokenStream2,
    r: &CTypeResolver,
) -> ffier_schema::Return {
    match ret {
        MetaReturn::Void => ffier_schema::Return::Void,
        MetaReturn::Value(tp) => {
            let rt = tokens_to_string(&tp.rust_type);
            ffier_schema::Return::Value(r.to_type_ref(&rt))
        }
        MetaReturn::Result { ok, err_ident } => {
            let ok_ref = ok.as_ref().map(|tp| {
                let rt = tokens_to_string(&tp.rust_type);
                r.to_type_ref(&rt)
            });
            ffier_schema::Return::Result {
                ok: ok_ref,
                err_type: err_ident.clone(),
            }
        }
    }
}

/// Convert a token stream to a string representation.
fn tokens_to_string(tokens: &TokenStream2) -> String {
    tokens.to_string()
}
