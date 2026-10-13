//! C header generator from ffier JSON schema.
//!
//! Reads a `ffier-{prefix}.json` and produces a complete C header with:
//! - Handle typedefs for types, errors, and traits
//! - Shared type infrastructure (Str, Bytes, Path, Result, macros)
//! - Error code `#define`s
//! - Function declarations with doc comments
//! - Destroy functions
//! - Trait impl bridge functions
//! - Self-dispatch functions for exported traits
//! - Error handle functions
//! - Utility functions (result_name)

use ffier_schema::{
    EnumType, ErrorType, ExportedType, FreeFunction, ImplementableTrait, Library, Method, Param,
    ParamType, Receiver, Return, ThreadSafety, TraitImpl, TypeKind, ValueStruct,
};

/// Options controlling C header generation.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct Options {
    /// Emit `typedef RET (*name_fn)(PARAMS);` for every function declaration.
    /// Useful for consumers that `dlopen`/`dlsym` at runtime.
    #[arg(long)]
    pub fn_typedefs: bool,
}

/// Generate a C header string from a library schema.
pub fn generate(lib: &Library, guard: &str, opts: &Options) -> String {
    let mut out = String::new();
    let fn_pfx = format!("{}_", lib.prefix);

    // Header guard + includes
    out.push_str(&format!("#ifndef {guard}\n"));
    out.push_str(&format!("#define {guard}\n\n"));
    out.push_str("#include <stddef.h>\n");
    out.push_str("#include <stdint.h>\n");
    out.push_str("#include <stdbool.h>\n");
    out.push_str("#include <string.h>\n");
    out.push_str("#ifndef _WIN32\n#include <sys/types.h>\n#endif\n");
    out.push('\n');

    // Handle typedefs — error types first.
    for err in &lib.errors {
        let c_name = lib.c_type_of(&err.name);
        out.push_str(&format!("typedef void* {};\n", c_name));
    }
    for ty in &lib.exported_types {
        let c_name = lib.c_type_of(&ty.name);
        out.push_str(&format!("typedef void* {};\n", c_name));
    }

    // Trait typedefs with implementor lists
    emit_trait_typedefs(&mut out, lib);

    // Foreign handle typedefs
    for entry in lib.type_registry.values() {
        if let ffier_schema::TypeKind::ForeignHandle { c_name, .. } = &entry.kind {
            out.push_str(&format!("typedef void* {c_name};\n"));
        }
    }
    out.push('\n');

    // Shared types — use primitives_prefix for type names and guards
    let prim_pfx = lib.primitives_prefix();
    let prim_upper_pfx = format!("{}_", prim_pfx.to_ascii_uppercase());
    emit_shared_types(&mut out, &prim_upper_pfx, &fn_pfx, lib, opts);

    emit_value_structs(&mut out, lib);

    // Enum constant sections
    for en in &lib.enum_constants {
        emit_enum_section(&mut out, en);
    }

    // Bitflags constant sections (same format as enum constants)
    for bf in &lib.bitflags_constants {
        emit_enum_section(&mut out, bf);
    }

    // Error sections
    for err in &lib.errors {
        emit_error_section(&mut out, err);
    }

    // Type sections (exported methods + destroy)
    for ty in &lib.exported_types {
        emit_type_section(&mut out, ty, lib, opts);
    }

    // Implementable traits: vtable struct + dispatch functions
    for tr in &lib.traits {
        if !tr.no_vtable {
            emit_vtable_section(&mut out, tr, lib);
        }
        emit_dispatch_section(&mut out, tr, lib, opts);
    }

    // Trait impl bridge functions
    for ti in &lib.trait_impls {
        emit_trait_impl_section(&mut out, ti, lib, opts);
    }

    // Free functions
    if !lib.free_functions.is_empty() {
        emit_free_functions(&mut out, &lib.free_functions, lib, opts);
    }

    // Utility functions
    emit_utility_functions(&mut out, &fn_pfx, lib, opts);

    out.push('\n');
    out.push_str(&format!("#endif /* {guard} */\n"));
    out
}

fn emit_value_structs(out: &mut String, lib: &Library) {
    fn visit<'a>(
        value: &'a ValueStruct,
        lib: &'a Library,
        visiting: &mut std::collections::HashSet<&'a str>,
        emitted: &mut std::collections::HashSet<&'a str>,
        ordered: &mut Vec<&'a ValueStruct>,
    ) {
        if emitted.contains(value.name.as_str()) || !visiting.insert(value.name.as_str()) {
            return;
        }
        for field in &value.fields {
            if lib
                .type_entry(&field.type_ref.type_name)
                .is_some_and(|entry| matches!(entry.kind, TypeKind::ValueStruct { .. }))
                && let Some(dependency) = lib
                    .value_structs
                    .iter()
                    .find(|candidate| candidate.name == field.type_ref.type_name)
            {
                visit(dependency, lib, visiting, emitted, ordered);
            }
        }
        visiting.remove(value.name.as_str());
        emitted.insert(value.name.as_str());
        ordered.push(value);
    }

    let mut visiting = std::collections::HashSet::new();
    let mut emitted = std::collections::HashSet::new();
    let mut ordered = Vec::new();
    for value in &lib.value_structs {
        visit(value, lib, &mut visiting, &mut emitted, &mut ordered);
    }
    if ordered.is_empty() {
        return;
    }
    emit_section_header(out, "Value structs");
    for value in ordered {
        out.push_str("typedef struct {\n");
        for field in &value.fields {
            out.push_str(&format!(
                "    {} {};\n",
                lib.c_type_of_ref(&field.type_ref),
                field.name
            ));
        }
        out.push_str(&format!("}} {};\n\n", value.c_name));
    }
}

/// Generate a C header from a JSON file path.
pub fn generate_from_file(
    json_path: &str,
    guard: &str,
    opts: &Options,
) -> Result<String, Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(json_path)?;
    let lib = Library::from_json(&json)?;
    Ok(generate(&lib, guard, opts))
}

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Emit shared primitive type definitions.
///
/// `prim_upper_pfx` is derived from `primitives_prefix` (e.g. `"KRUN_"`).
/// `fn_pfx` is derived from the library prefix (e.g. `"krun_init_"`) and
/// is used for `str_free` — that function routes through a library-specific
/// allocator and must NOT share the primitives prefix.
///
/// Each primitive type definition is guarded by `#ifndef` so that when
/// multiple headers share the same `primitives_prefix`, whichever is
/// included first defines the types.
fn emit_shared_types(
    out: &mut String,
    prim_upper_pfx: &str,
    fn_pfx: &str,
    lib: &Library,
    opts: &Options,
) {
    use ffier_schema::Blessing;
    let result_c = blessed_c_type(lib, Blessing::Result);
    let str_c = blessed_c_type(lib, Blessing::Str);
    let bytes_c = try_blessed_c_type(lib, Blessing::Bytes);
    let path_c = try_blessed_c_type(lib, Blessing::Path);
    let vtable_handle_c = blessed_c_type(lib, Blessing::VtableHandle);
    let result_success = format!("{prim_upper_pfx}RESULT_SUCCESS");
    let str_macro = format!("{prim_upper_pfx}STR");

    // Guard: all primitive typedefs share one guard keyed to the primitives prefix
    let prim_guard = format!("{prim_upper_pfx}PRIMITIVES_DEFINED");
    out.push_str(&format!("#ifndef {prim_guard}\n"));
    out.push_str(&format!("#define {prim_guard}\n\n"));

    // FIXME: The struct layouts are still hardcoded. The type registry
    // carries the C type names but not field layouts. These should be
    // driven by the schema — e.g. TypeKind::Struct { fields: [...] }
    // — so generators don't need to hardcode ABI details.
    // Object — generic handle type, any concrete handle can be used where
    // FtObject is expected.
    let object_c = blessed_c_type(lib, Blessing::Object);
    let all_handles: Vec<&str> = lib
        .errors
        .iter()
        .map(|e| lib.c_type_of(&e.name))
        .chain(lib.exported_types.iter().map(|t| lib.c_type_of(&t.name)))
        .collect();
    let handles_list = all_handles.join(" | ");
    out.push_str(&format!(
        "typedef void* {object_c}; /* {handles_list} */\n\n"
    ));

    out.push_str(&format!("typedef uint64_t {result_c};\n"));
    out.push_str(&format!("#define {result_success} 0\n\n"));
    out.push_str("/* Caller must ensure data is valid UTF-8 */\n");
    out.push_str("typedef struct {\n");
    out.push_str("    const char* data;\n");
    out.push_str("    size_t len;\n");
    out.push_str(&format!("}} {str_c};\n\n"));
    if let Some(bytes_c) = &bytes_c {
        out.push_str("typedef struct {\n");
        out.push_str("    const uint8_t* data;\n");
        out.push_str("    size_t len;\n");
        out.push_str(&format!("}} {bytes_c};\n\n"));
        out.push_str("/* Writable initialized bytes; len must not exceed Rust's isize::MAX.\n");
        out.push_str(
            " * For nonempty buffers, data must be non-null and valid for reads and writes.\n",
        );
        out.push_str(
            " * Keep the buffer valid and exclusively accessible for the call and for every\n",
        );
        out.push_str(
            " * active Rust borrow, including any borrow returned or retained by the API.\n",
        );
        out.push_str(" * A zero-length buffer may have a null data pointer. */\n");
        out.push_str("typedef struct {\n");
        out.push_str("    uint8_t* data;\n");
        out.push_str("    size_t len;\n");
        out.push_str(&format!("}} {bytes_c}Mut;\n\n"));
        if let Some(path_c) = &path_c {
            out.push_str("/* OS path — arbitrary bytes on Unix, not necessarily UTF-8 */\n");
            out.push_str(&format!("typedef {bytes_c} {path_c};\n\n"));
        }
    } else if let Some(path_c) = &path_c {
        out.push_str("/* OS path — arbitrary bytes on Unix, not necessarily UTF-8 */\n");
        out.push_str("typedef struct {\n");
        out.push_str("    const uint8_t* data;\n");
        out.push_str("    size_t len;\n");
        out.push_str(&format!("}} {path_c};\n\n"));
    }
    out.push_str(&format!(
        "#define {str_macro}(s) (({str_c}){{ .data = (s), .len = (s) ? strlen(s) : 0 }})\n"
    ));
    if let Some(bytes_c) = &bytes_c {
        let bytes_macro = format!("{prim_upper_pfx}BYTES");
        out.push_str("#if defined(__GNUC__)\n");
        out.push_str(&format!("#define {bytes_macro}(arr) ({{ \\\n"));
        out.push_str("    _Static_assert( \\\n");
        out.push_str("        !__builtin_types_compatible_p(typeof(arr), typeof(&(arr)[0])), \\\n");
        out.push_str(&format!(
            "        \"{bytes_macro}() requires an array, not a pointer\"); \\\n"
        ));
        out.push_str(&format!(
            "    (({bytes_c}){{ .data = (const uint8_t*)(arr), .len = sizeof(arr) }}); \\\n"
        ));
        out.push_str("})\n");
        out.push_str("#else\n");
        out.push_str(&format!("#define {bytes_macro}(arr) \\\n"));
        out.push_str(&format!(
            "    (({bytes_c}){{ .data = (const uint8_t*)(arr), .len = sizeof(arr) }})\n"
        ));
        out.push_str("#endif\n\n");
    }

    // FIXME: This struct layout is hardcoded — should come from the schema.
    let vtable_handle_macro = format!("{prim_upper_pfx}VTABLE_HANDLE");
    out.push_str("/**\n");
    out.push_str(" * Stack-allocated temporary handle for passing vtable-based objects.\n");
    out.push_str(" * Only valid for the duration of the call — the callee borrows, not owns.\n");
    out.push_str(
        " * Use only for borrowed callback parameters. Never pass it to an owned parameter.\n",
    );
    out.push_str(" * Borrowed callbacks cannot be retained, destroyed, or used after the call.\n");
    out.push_str(
        " * Owned callbacks require a heap handle with ffier's owned layout and transfer ownership.\n",
    );
    out.push_str(
        " * Allocation must be compatible with Rust's allocator; never transfer a stack handle.\n",
    );
    out.push_str(
        " * The caller must not use or destroy a transferred handle. The callee destroys\n",
    );
    out.push_str(
        " * it exactly once when released, calling the vtable drop callback once if present.\n",
    );
    out.push_str(
        " * Keep vtable and user_data valid until that destruction, not merely call return.\n",
    );
    out.push_str(
        " * A + 'static API requires callback data with no expiring borrows. Other lifetime\n",
    );
    out.push_str(
        " * bounds require release before borrowed data expires. C must enforce these bounds.\n",
    );
    out.push_str(
        " * Thread guarantees depend on the exported trait, not this raw handle layout.\n",
    );
    out.push_str(
        " * Send permits owned/mutable callback transfer and destruction on another thread.\n",
    );
    out.push_str(" * Sync permits concurrent shared callbacks. Mutable access always requires exclusivity.\n");
    out.push_str(" * Vtable storage, user_data, callback functions, and drop must uphold those guarantees.\n");
    out.push_str(" * Vtable storage remains valid and immutable throughout callback use.\n");
    out.push_str(
        " * Shared borrowed callbacks may transfer with Sync; Send alone is insufficient.\n",
    );
    out.push_str(" */\n");
    out.push_str("typedef struct {\n");
    out.push_str("    uint32_t type_tag;\n");
    out.push_str("    uint32_t metadata;\n");
    out.push_str("    const void *vtable_ptr;\n");
    out.push_str("    const void *user_data;\n");
    out.push_str("    uint16_t vtable_size;\n");
    out.push_str(&format!("}} {vtable_handle_c};\n\n"));
    out.push_str(&format!(
        "#define {vtable_handle_macro}(tag, vtable, self_data) \\\n"
    ));
    out.push_str(&format!(
        "    (({vtable_handle_c}){{ .type_tag = (tag), .metadata = 0, \\\n"
    ));
    out.push_str("      .vtable_ptr = &(vtable), .user_data = (self_data), \\\n");
    out.push_str("      .vtable_size = sizeof(vtable) })\n\n");

    // ObjectArrayEntry — opaque 16-byte element matching FfierBorrowedHandle layout.
    // ObjectArray — contiguous array of these entries.
    if let Some(object_array_c) = try_blessed_c_type(lib, Blessing::ObjectArray) {
        // Derive PascalCase prefix from the ObjectArray c_type (e.g. "FtObjectArray" → "Ft")
        let type_pfx = object_array_c
            .strip_suffix("ObjectArray")
            .unwrap_or(&object_array_c);
        let entry_c = format!("{type_pfx}ObjectArrayEntry");

        out.push_str("/**\n");
        out.push_str(" * Opaque 16-byte handle entry in an object array.\n");
        out.push_str(" * Do not access fields directly — pass &entry to typed methods.\n");
        out.push_str(" * Do NOT pass individual entries to destroy.\n");
        out.push_str(" */\n");
        out.push_str("typedef struct {\n");
        out.push_str("    uint32_t _tag;\n");
        out.push_str("    uint32_t _meta;\n");
        out.push_str("    const void* _ptr;\n");
        out.push_str(&format!("}} {entry_c};\n\n"));

        out.push_str("/**\n");
        out.push_str(" * Contiguous array of borrowed handles.\n");
        out.push_str(" * Returned by methods that produce slices of handles.\n");
        out.push_str(" * Individual elements must NOT be passed to destroy —\n");
        out.push_str(" * call free_object_array() to free the entire array.\n");
        out.push_str(" */\n");
        out.push_str("typedef struct {\n");
        out.push_str(&format!("    const {entry_c}* items;\n"));
        out.push_str("    size_t len;\n");
        out.push_str(&format!("}} {object_array_c};\n\n"));

        // FT_OBJECT_ARRAY_GET macro — inline element access, returns FtObject.
        // Bounds-checked via assert (compiled out in release with NDEBUG).
        let get_macro = format!("{}_OBJECT_ARRAY_GET", prim_upper_pfx.trim_end_matches('_'));
        out.push_str(&format!(
            "#define {get_macro}(arr, i) \\\n    (assert((size_t)(i) < (arr).len), ({object_c})(void*)&(arr).items[(i)])\n\n"
        ));
    }

    out.push_str(&format!("#endif /* {prim_guard} */\n\n"));

    // str_free — uses the library prefix, NOT the primitives prefix.
    // Each library has its own allocator so str_free must be library-specific.
    let str_free_fn = format!("{fn_pfx}str_free");
    out.push_str("/* Free an owned string returned by the library */\n");
    write_decl(out, "void", &str_free_fn, &format!("{str_c} s"), opts);
    out.push('\n');

    // free_object_array — uses the library prefix (each library has its own allocator).
    if let Some(object_array_c) = try_blessed_c_type(lib, Blessing::ObjectArray) {
        let free_fn = format!("{fn_pfx}free_object_array");
        out.push_str("/* Free an object array returned by the library */\n");
        write_decl(out, "void", &free_fn, &format!("{object_array_c} a"), opts);
        out.push('\n');
    }
}

// ---------------------------------------------------------------------------
// Enum constants
// ---------------------------------------------------------------------------

fn emit_enum_section(out: &mut String, en: &EnumType) {
    emit_section_header(out, &en.name);
    for v in &en.variants {
        out.push_str(&format!("#define {} {}\n", v.c_name, v.value));
    }
}

// ---------------------------------------------------------------------------
// Error code constants
// ---------------------------------------------------------------------------

fn emit_error_section(out: &mut String, err: &ErrorType) {
    emit_section_header(out, &err.name);
    let type_tag = err.type_tag;
    for v in &err.variants {
        out.push_str(&format!(
            "#define {} ((uint64_t){} << 32 | {})\n",
            v.c_name, type_tag, v.code,
        ));
    }
}

// ---------------------------------------------------------------------------
// Trait typedefs with implementor lists
// ---------------------------------------------------------------------------

fn emit_trait_typedefs(out: &mut String, lib: &Library) {
    // Build a map: trait_name → vec of implementor C names
    let mut trait_implementors: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for ti in &lib.trait_impls {
        let implementor_c = find_type_c_name(lib, &ti.struct_name);
        trait_implementors
            .entry(ti.trait_name.clone())
            .or_default()
            .push(implementor_c);
    }

    // For exported traits with vtables, also add the VtableXxx wrapper as an implementor
    for tr in &lib.traits {
        if !tr.no_vtable {
            trait_implementors
                .entry(tr.name.clone())
                .or_default()
                .push(tr.wrapper_c_name.clone());
        }
    }

    // Collect all trait names that need typedefs (from both implementables and trait_impls)
    let mut trait_names: Vec<String> = trait_implementors.keys().cloned().collect();
    trait_names.sort();

    for trait_name in &trait_names {
        let c_name = lib.c_type_of(trait_name);
        let implementors = &trait_implementors[trait_name];
        let list = implementors.join(" | ");
        out.push_str(&format!("typedef void* {c_name}; /* {list} */\n"));
    }
}

// ---------------------------------------------------------------------------
// Vtable struct definitions
// ---------------------------------------------------------------------------

fn emit_vtable_section(out: &mut String, tr: &ImplementableTrait, lib: &Library) {
    let vtable_name = &tr.vtable_struct_c_name;

    emit_section_header(out, vtable_name);
    emit_thread_safety_contract(out, tr.thread_safety);

    // Type tag constant for constructing vtable handles from C.
    if let Some(entry) = lib.type_entry(&tr.name)
        && let Some(tag) = entry.type_tag
    {
        out.push_str(&format!("#define {} {}\n\n", tr.type_tag_constant, tag));
    }

    out.push_str("typedef struct {\n");

    // Slot 0 is always drop
    out.push_str("    /* Releases transferred user_data exactly once; never called for borrowed parameters. */\n");
    out.push_str("    void (*drop)(void* self_data);\n");

    // Build a map of index → method for gap detection
    let mut method_by_index: std::collections::HashMap<usize, &Method> =
        std::collections::HashMap::new();
    for m in &tr.methods {
        if let Some(td) = &m.trait_definition {
            method_by_index.insert(td.index, m);
        }
    }

    // Emit method slots 0..=max_vtable_slot (after drop at struct position 0).
    // Method index N occupies struct position N+1.
    for slot in 0..=tr.max_vtable_slot {
        if let Some(m) = method_by_index.get(&slot) {
            // Build the function pointer signature
            let mut params = vec!["void* self_data".to_string()];
            format_c_params(&m.params, lib, &mut params);

            let (ret_type, extra_params) = format_return_and_out_params(&m.ret, false, lib);
            params.extend(extra_params);

            let params_str = params.join(", ");
            out.push_str(&format!("    {ret_type} (*{})({params_str});\n", m.name));
        } else {
            // Reserved/retired slot
            out.push_str(&format!(
                "    void (*__reserved_{slot})(void); /* reserved slot {slot} */\n"
            ));
        }
    }

    out.push_str(&format!("}} {vtable_name};\n"));
}

// ---------------------------------------------------------------------------
// Exported type methods + destroy
// ---------------------------------------------------------------------------

fn emit_type_section(out: &mut String, ty: &ExportedType, lib: &Library, opts: &Options) {
    emit_section_header(out, &ty.name);
    emit_thread_safety_contract(out, ty.thread_safety);

    let c_name = lib.c_type_of(&ty.name);

    for m in &ty.methods {
        emit_doc_comment(out, &m.doc);
        let (ret_type, params_str) = build_c_declaration_parts(c_name, m, ty.is_builder_type, lib);
        write_decl(out, &ret_type, &m.ffi_name, &params_str, opts);
    }

    // Destroy function
    write_decl(
        out,
        "void",
        &ty.destroy_ffi_name,
        &format!("{c_name} handle"),
        opts,
    );
}

fn emit_thread_safety_contract(out: &mut String, thread_safety: ThreadSafety) {
    if thread_safety.is_empty() {
        return;
    }
    out.push_str("/* Thread-safety contract:\n");
    if thread_safety.send {
        out.push_str(" * Send: ownership and mutable borrows may transfer between threads; destruction may run there.\n");
    }
    if thread_safety.sync {
        out.push_str(" * Sync: shared access may run concurrently across threads.\n");
    }
    out.push_str(" * Mutable access and destruction require exclusive access. Lifetime bounds still apply.\n */\n");
}

// ---------------------------------------------------------------------------
// Trait impl bridge functions
// ---------------------------------------------------------------------------

fn emit_trait_impl_section(out: &mut String, ti: &TraitImpl, lib: &Library, opts: &Options) {
    let struct_c_name = find_type_c_name(lib, &ti.struct_name);

    for m in &ti.methods {
        let (ret_type, params_str) = build_dispatch_declaration_parts(&struct_c_name, m, lib);
        write_decl(out, &ret_type, &m.ffi_name, &params_str, opts);
    }
}

// ---------------------------------------------------------------------------
// Self-dispatch functions for exported traits
// ---------------------------------------------------------------------------

fn emit_dispatch_section(out: &mut String, tr: &ImplementableTrait, lib: &Library, opts: &Options) {
    emit_section_header(out, &format!("{} (dispatch)", tr.name));

    let handle_c_name = lib.c_type_of(&tr.name);

    // Only emit dispatch for own methods (not supertrait methods)
    for m in tr.methods.iter().take(tr.own_method_count) {
        emit_doc_comment(out, &m.doc);
        let (ret_type, params_str) = build_dispatch_declaration_parts(handle_c_name, m, lib);
        write_decl(out, &ret_type, &m.ffi_name, &params_str, opts);
    }

    // Destroy dispatch
    write_decl(
        out,
        "void",
        &tr.destroy_ffi_name,
        &format!("{handle_c_name} handle"),
        opts,
    );
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

fn emit_free_functions(
    out: &mut String,
    functions: &[FreeFunction],
    lib: &Library,
    opts: &Options,
) {
    emit_section_header(out, "Free functions");

    for f in functions {
        emit_doc_comment(out, &f.doc);

        let mut params: Vec<String> = Vec::new();
        format_c_params(&f.params, lib, &mut params);

        let (ret_type, extra_params) = format_return_and_out_params(&f.ret, false, lib);
        params.extend(extra_params);

        let params_str = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };
        write_decl(out, &ret_type, &f.ffi_name, &params_str, opts);
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn emit_utility_functions(out: &mut String, fn_pfx: &str, lib: &Library, opts: &Options) {
    let result_c = blessed_c_type(lib, ffier_schema::Blessing::Result);
    if !lib.errors.is_empty() {
        out.push_str("/* Create an owned fieldless error; returns NULL for payload variants or unknown codes.\n");
        out.push_str(
            " * Destroy once with error_destroy or transfer ownership to a callback caller. */\n",
        );
        write_decl(
            out,
            "void*",
            &format!("{fn_pfx}error_new"),
            &format!("{result_c} r"),
            opts,
        );
        for err in &lib.errors {
            for variant in &err.variants {
                if let Some(factory) = &variant.constructor_ffi_name {
                    out.push_str("/* Construct an owned error. Handle fields transfer ownership; strings are copied. */\n");
                    let params = variant
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(i, field)| {
                            format!("{} field_{i}", lib.c_type_of_ref(&field.type_ref))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    write_decl(out, "void*", factory, &params, opts);
                }
            }
        }
        out.push_str("/* Borrow an indexed payload field into an aligned buffer. Invalid indices leave it untouched.\n");
        out.push_str(" * The data must not outlive the error. Handle fields return borrowed shells: destroy each shell\n");
        out.push_str(
            " * before destroying the error, without consuming or mutating the field. */\n",
        );
        write_decl(
            out,
            "void",
            &format!("{fn_pfx}error_field"),
            "const void* handle, size_t index, void* out_buf, size_t buf_size",
            opts,
        );
        out.push_str("/* Equivalent to error_field(handle, 0, out_buf, buf_size). */\n");
        write_decl(
            out,
            "void",
            &format!("{fn_pfx}error_payload"),
            "const void* handle, void* out_buf, size_t buf_size",
            opts,
        );
    }
    let str_c = blessed_c_type(lib, ffier_schema::Blessing::Str);
    let result_name_fn = format!("{fn_pfx}result_name");
    let result_name_cstr_fn = format!("{fn_pfx}result_name_cstr");
    write_decl(out, &str_c, &result_name_fn, &format!("{result_c} r"), opts);
    write_decl(
        out,
        "const char*",
        &result_name_cstr_fn,
        &format!("{result_c} r"),
        opts,
    );
}

// ---------------------------------------------------------------------------
// Param formatting (shared across all declaration types)
// ---------------------------------------------------------------------------

/// Append C parameter strings for a list of schema params.
/// Uses `c_params` from the schema for Slice types (no guessing).
fn format_c_params(params: &[Param], lib: &Library, out: &mut Vec<String>) {
    for p in params {
        match &p.param_type {
            ParamType::Regular(type_ref) => {
                let c_type = lib.c_type_of_ref(type_ref);
                out.push(format!("{} {}", c_type, p.name));
            }
            ParamType::Slice { c_params, .. } => {
                for cp in c_params {
                    out.push(format!("{} {}", cp.c_type, cp.name));
                }
            }
            ParamType::ImplTrait {
                trait_name,
                type_args,
                lifetime_bounds,
                ref_kind,
                ref_lifetime,
                ..
            } => {
                let c_type = lib.c_type_of(trait_name);
                let ownership = match ref_kind {
                    ffier_schema::RefKind::None => "owned callback; valid until release",
                    ffier_schema::RefKind::Shared => "shared borrowed callback; call-scoped",
                    ffier_schema::RefKind::Mut => "mutable borrowed callback; call-scoped",
                };
                let mut requirements = vec![ownership.to_string()];
                if let Some(tr) = lib.traits.iter().find(|tr| &tr.name == trait_name) {
                    if tr.thread_safety.send {
                        requirements.push("Send".to_string());
                    }
                    if tr.thread_safety.sync {
                        requirements.push("Sync".to_string());
                    }
                }
                for (label, lifetimes) in [
                    ("trait arguments", type_args),
                    ("value bounds", lifetime_bounds),
                ] {
                    if !lifetimes.is_empty() {
                        let lifetimes = lifetimes
                            .iter()
                            .map(|lt| format!("'{lt}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        requirements.push(format!("{label}: {lifetimes}"));
                    }
                }
                if let Some(lifetime) = ref_lifetime {
                    requirements.push(format!("reference lifetime: '{lifetime}"));
                }
                out.push(format!(
                    "{} {} /* {} */",
                    c_type,
                    p.name,
                    requirements.join("; ")
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Declaration formatting
// ---------------------------------------------------------------------------

/// Build the components of a C function declaration for an exported method.
/// Returns `(return_type, params_str)`.
fn build_c_declaration_parts(
    handle_c_name: &str,
    m: &Method,
    is_builder_type: bool,
    lib: &Library,
) -> (String, String) {
    let mut params: Vec<String> = Vec::new();

    // Handle param (self)
    match m.receiver {
        Receiver::None => {}
        Receiver::Value if is_builder_type => {
            params.push(format!("{handle_c_name}* handle"));
        }
        Receiver::Ref | Receiver::Mut | Receiver::Value => {
            params.push(format!("{handle_c_name} handle"));
        }
    }

    format_c_params(&m.params, lib, &mut params);

    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    let (ret_type, extra_params) = format_return_and_out_params(&m.ret, is_builder, lib);
    params.extend(extra_params);

    let params_str = params.join(", ");
    (ret_type, params_str)
}

/// Build the components of a C function declaration for a trait dispatch or trait impl method.
/// Returns `(return_type, params_str)`.
fn build_dispatch_declaration_parts(
    handle_c_name: &str,
    m: &Method,
    lib: &Library,
) -> (String, String) {
    let mut params = vec![format!("{handle_c_name} handle")];
    format_c_params(&m.params, lib, &mut params);

    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    let (ret_type, extra_params) = format_return_and_out_params(&m.ret, is_builder, lib);
    params.extend(extra_params);

    let params_str = params.join(", ");
    (ret_type, params_str)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a C function declaration, and optionally a `_fn` function-pointer typedef.
fn write_decl(out: &mut String, ret_type: &str, fn_name: &str, params_str: &str, opts: &Options) {
    out.push_str(&format!("{ret_type} {fn_name}({params_str});\n"));
    if opts.fn_typedefs {
        out.push_str(&format!(
            "typedef {ret_type} (*{fn_name}_fn)({params_str});\n"
        ));
    }
}

/// Compute the C return type and any extra out-parameters for a method.
/// Returns `(c_return_type, extra_params_to_append)`.
fn format_return_and_out_params(
    ret: &Return,
    is_builder: bool,
    lib: &Library,
) -> (String, Vec<String>) {
    let result_c = || blessed_c_type(lib, ffier_schema::Blessing::Result);
    match ret {
        Return::Void => ("void".to_string(), vec![]),
        Return::Value(_) if is_builder => ("void".to_string(), vec![]),
        Return::Value(type_ref) => (lib.c_type_of_ref(type_ref), vec![]),
        Return::OptionalValue { value } => (
            "bool".to_string(),
            vec![format!("{}* result", lib.c_type_of_ref(value))],
        ),
        Return::Result {
            ok, c_convention, ..
        } => {
            let error_c = blessed_c_type(lib, ffier_schema::Blessing::ErrorTrait);
            use ffier_schema::CResultConvention;
            match c_convention {
                CResultConvention::HandleOrNull => {
                    // GLib-style: return handle pointer, NULL on error
                    let ok_ref = ok.as_ref().expect("HandleOrNull requires an ok type");
                    (
                        lib.c_type_of(&ok_ref.type_name).to_string(),
                        vec![format!("{error_c}* err_out")],
                    )
                }
                CResultConvention::OutParam => match ok {
                    None | Some(_) if is_builder => {
                        (result_c(), vec![format!("{error_c}* err_out")])
                    }
                    Some(ok_ref) => {
                        let c_type = lib.c_type_of_ref(ok_ref);
                        (
                            result_c(),
                            vec![format!("{}* result", c_type), format!("{error_c}* err_out")],
                        )
                    }
                    None => (result_c(), vec![format!("{error_c}* err_out")]),
                },
                CResultConvention::OptionalValueOutParam => {
                    let ok_ref = ok
                        .as_ref()
                        .expect("OptionalValueOutParam requires an ok type");
                    (
                        result_c(),
                        vec![
                            "bool* result_is_some".to_string(),
                            format!("{}* result", lib.c_type_of_ref(ok_ref)),
                            format!("{error_c}* err_out"),
                        ],
                    )
                }
            }
        }
        Return::ObjectArray { .. } => {
            let object_array_c = blessed_c_type(lib, ffier_schema::Blessing::ObjectArray);
            (object_array_c.to_string(), vec![])
        }
    }
}

fn emit_section_header(out: &mut String, name: &str) {
    let dashes = "-".repeat(72usize.saturating_sub(6 + name.len()));
    out.push_str(&format!("\n/* {name} {dashes} */\n\n"));
}

fn emit_doc_comment(out: &mut String, doc: &[String]) {
    if doc.is_empty() {
        return;
    }
    if doc.len() == 1 {
        out.push_str(&format!("/**{} */\n", doc[0]));
    } else {
        out.push_str("/**\n");
        for line in doc {
            out.push_str(&format!(" *{line}\n"));
        }
        out.push_str(" */\n");
    }
}

fn find_type_c_name(lib: &Library, name: &str) -> String {
    lib.c_type_of(name).to_string()
}

fn blessed_c_type(lib: &Library, tag: ffier_schema::Blessing) -> String {
    let (name, _) = lib
        .blessed(tag)
        .unwrap_or_else(|| panic!("no type blessed as {tag:?} found in schema"));
    lib.c_type_of(name).to_string()
}

fn try_blessed_c_type(lib: &Library, tag: ffier_schema::Blessing) -> Option<String> {
    lib.blessed(tag)
        .map(|(name, _)| lib.c_type_of(name).to_string())
}
