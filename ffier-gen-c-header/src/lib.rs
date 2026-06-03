//! C header generator from ffier JSON schema.
//!
//! Reads a `ffier-{prefix}.json` and produces a complete C header with:
//! - Handle typedefs for types, errors, and traits
//! - Shared type infrastructure (Str, Bytes, Path, Result, macros)
//! - Error code `#define`s
//! - Function declarations with doc comments
//! - Destroy functions
//! - Trait impl bridge functions
//! - Self-dispatch functions for implementable traits
//! - Error handle functions
//! - Utility functions (result_name)

use ffier_schema::{
    EnumType, ErrorType, ExportedType, FreeFunction, ImplementableTrait, Library, Method,
    MethodContext, Param, ParamType, Receiver, Return, TraitImpl,
};

/// Generate a C header string from a library schema.
pub fn generate(lib: &Library, guard: &str) -> String {
    let mut out = String::new();
    let fn_pfx = format!("{}_", lib.prefix);

    // Header guard + includes
    out.push_str(&format!("#ifndef {guard}\n"));
    out.push_str(&format!("#define {guard}\n\n"));
    out.push_str("#include <stddef.h>\n");
    out.push_str("#include <stdint.h>\n");
    out.push_str("#include <stdbool.h>\n");
    out.push_str("#include <string.h>\n\n");

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
    out.push('\n');

    // Shared types — use primitives_prefix for type names and guards
    let prim_pfx = lib.primitives_prefix();
    let prim_upper_pfx = format!("{}_", prim_pfx.to_ascii_uppercase());
    emit_shared_types(&mut out, &prim_upper_pfx, &fn_pfx, lib);

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
        emit_error_section(&mut out, err, lib);
    }

    // Type sections (exportable methods + destroy)
    for ty in &lib.exported_types {
        emit_type_section(&mut out, ty, lib);
    }

    // Implementable traits: vtable struct + dispatch functions
    for tr in &lib.traits {
        emit_vtable_section(&mut out, tr, lib);
        emit_dispatch_section(&mut out, tr, lib);
    }

    // Trait impl bridge functions
    for ti in &lib.trait_impls {
        emit_trait_impl_section(&mut out, ti, lib);
    }

    // Free functions
    if !lib.free_functions.is_empty() {
        emit_free_functions(&mut out, &lib.free_functions, lib);
    }

    // Utility functions
    emit_utility_functions(&mut out, &fn_pfx, lib);

    out.push('\n');
    out.push_str(&format!("#endif /* {guard} */\n"));
    out
}

/// Generate a C header from a JSON file path.
pub fn generate_from_file(
    json_path: &str,
    guard: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(json_path)?;
    let lib = Library::from_json(&json)?;
    Ok(generate(&lib, guard))
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
fn emit_shared_types(out: &mut String, prim_upper_pfx: &str, fn_pfx: &str, lib: &Library) {
    use ffier_schema::Blessing;
    let result_c = blessed_c_type(lib, Blessing::Result);
    let str_c = blessed_c_type(lib, Blessing::Str);
    let bytes_c = blessed_c_type(lib, Blessing::Bytes);
    let path_c = try_blessed_c_type(lib, Blessing::Path);
    let vtable_handle_c = blessed_c_type(lib, Blessing::VtableHandle);
    let result_success = format!("{prim_upper_pfx}RESULT_SUCCESS");
    let str_macro = format!("{prim_upper_pfx}STR");
    let bytes_macro = format!("{prim_upper_pfx}BYTES");

    // Guard: all primitive typedefs share one guard keyed to the primitives prefix
    let prim_guard = format!("{prim_upper_pfx}PRIMITIVES_DEFINED");
    out.push_str(&format!("#ifndef {prim_guard}\n"));
    out.push_str(&format!("#define {prim_guard}\n\n"));

    // FIXME: The struct layouts are still hardcoded. The type registry
    // carries the C type names but not field layouts. These should be
    // driven by the schema — e.g. TypeKind::Struct { fields: [...] }
    // — so generators don't need to hardcode ABI details.
    out.push_str(&format!("typedef uint64_t {result_c};\n"));
    out.push_str(&format!("#define {result_success} 0\n\n"));
    out.push_str("/* Caller must ensure data is valid UTF-8 */\n");
    out.push_str("typedef struct {\n");
    out.push_str("    const char* data;\n");
    out.push_str("    size_t len;\n");
    out.push_str(&format!("}} {str_c};\n\n"));
    out.push_str("typedef struct {\n");
    out.push_str("    const uint8_t* data;\n");
    out.push_str("    size_t len;\n");
    out.push_str(&format!("}} {bytes_c};\n\n"));
    if let Some(path_c) = &path_c {
        out.push_str("/* OS path — arbitrary bytes on Unix, not necessarily UTF-8 */\n");
        out.push_str(&format!("typedef {bytes_c} {path_c};\n\n"));
    }
    out.push_str(&format!(
        "#define {str_macro}(s) (({str_c}){{ .data = (s), .len = (s) ? strlen(s) : 0 }})\n"
    ));
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

    // FIXME: This struct layout is hardcoded — should come from the schema.
    let vtable_handle_macro = format!("{prim_upper_pfx}VTABLE_HANDLE");
    out.push_str("/**\n");
    out.push_str(" * Stack-allocated temporary handle for passing vtable-based objects.\n");
    out.push_str(" * Only valid for the duration of the call — the callee borrows, not owns.\n");
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

    out.push_str(&format!("#endif /* {prim_guard} */\n\n"));

    // str_free — uses the library prefix, NOT the primitives prefix.
    // Each library has its own allocator so str_free must be library-specific.
    let str_free_fn = format!("{fn_pfx}str_free");
    out.push_str("/* Free an owned string returned by the library */\n");
    out.push_str(&format!("void {str_free_fn}({str_c} s);\n\n"));
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

fn emit_error_section(out: &mut String, err: &ErrorType, lib: &Library) {
    emit_section_header(out, &err.name);
    let type_tag = lib.type_registry[&err.name]
        .type_tag
        .expect("error type must have a type_tag");
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

    // For implementable traits, also add the VtableXxx wrapper as an implementor
    for tr in &lib.traits {
        trait_implementors
            .entry(tr.name.clone())
            .or_default()
            .push(tr.wrapper_c_name.clone());
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

    // Type tag constant for constructing vtable handles from C.
    if let Some(entry) = lib.type_entry(&tr.name) {
        if let Some(tag) = entry.type_tag {
            out.push_str(&format!("#define {} {}\n\n", tr.type_tag_constant, tag));
        }
    }

    out.push_str("typedef struct {\n");

    // Slot 0 is always drop
    out.push_str("    void (*drop)(void* self_data);\n");

    // Build a map of index → method for gap detection
    let mut method_by_index: std::collections::HashMap<usize, &Method> =
        std::collections::HashMap::new();
    for m in &tr.methods {
        if let MethodContext::Trait { index, .. } = &m.context {
            method_by_index.insert(*index, m);
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

fn emit_type_section(out: &mut String, ty: &ExportedType, lib: &Library) {
    emit_section_header(out, &ty.name);

    let c_name = lib.c_type_of(&ty.name);

    for m in &ty.methods {
        let MethodContext::Exportable { ffi_name } = &m.context else {
            continue;
        };
        emit_doc_comment(out, &m.doc);
        let decl = format_c_declaration(ffi_name, c_name, m, ty.is_builder_type, lib);
        out.push_str(&decl);
        out.push('\n');
    }

    // Destroy function
    out.push_str(&format!(
        "void {}({} handle);\n",
        ty.destroy_ffi_name, c_name
    ));
}

// ---------------------------------------------------------------------------
// Trait impl bridge functions
// ---------------------------------------------------------------------------

fn emit_trait_impl_section(out: &mut String, ti: &TraitImpl, lib: &Library) {
    let struct_c_name = find_type_c_name(lib, &ti.struct_name);

    for m in &ti.methods {
        let MethodContext::Trait { ffi_name, .. } = &m.context else {
            continue;
        };
        let decl = format_dispatch_declaration(ffi_name, &struct_c_name, m, lib);
        out.push_str(&decl);
        out.push('\n');
    }
}

// ---------------------------------------------------------------------------
// Self-dispatch functions for implementable traits
// ---------------------------------------------------------------------------

fn emit_dispatch_section(out: &mut String, tr: &ImplementableTrait, lib: &Library) {
    emit_section_header(out, &format!("{} (dispatch)", tr.name));

    let handle_c_name = lib.c_type_of(&tr.name);

    // Only emit dispatch for own methods (not supertrait methods)
    for m in tr.methods.iter().take(tr.own_method_count) {
        let MethodContext::Trait { ffi_name, .. } = &m.context else {
            continue;
        };
        emit_doc_comment(out, &m.doc);
        let decl = format_dispatch_declaration(ffi_name, handle_c_name, m, lib);
        out.push_str(&decl);
        out.push('\n');
    }

    // Destroy dispatch
    out.push_str(&format!(
        "void {}({} handle);\n",
        tr.destroy_ffi_name, handle_c_name
    ));
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

fn emit_free_functions(out: &mut String, functions: &[FreeFunction], lib: &Library) {
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
        out.push_str(&format!("{ret_type} {}({params_str});\n", f.ffi_name));
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn emit_utility_functions(out: &mut String, fn_pfx: &str, lib: &Library) {
    let result_c = blessed_c_type(lib, ffier_schema::Blessing::Result);
    let str_c = blessed_c_type(lib, ffier_schema::Blessing::Str);
    out.push_str(&format!("{str_c} {fn_pfx}result_name({result_c} r);\n"));
    out.push_str(&format!(
        "const char* {fn_pfx}result_name_cstr({result_c} r);\n"
    ));
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
                let c_type = lib.c_type_of(&type_ref.type_name);
                out.push(format!("{} {}", c_type, p.name));
            }
            ParamType::Slice { c_params, .. } => {
                for cp in c_params {
                    out.push(format!("{} {}", cp.c_type, cp.name));
                }
            }
            ParamType::ImplTrait { trait_name, .. } => {
                let c_type = lib.c_type_of(trait_name);
                out.push(format!("{} {}", c_type, p.name));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Declaration formatting
// ---------------------------------------------------------------------------

/// Format a C function declaration for an exportable method.
fn format_c_declaration(
    ffi_name: &str,
    handle_c_name: &str,
    m: &Method,
    is_builder_type: bool,
    lib: &Library,
) -> String {
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
    format!("{ret_type} {ffi_name}({params_str});")
}

/// Format a C function declaration for a trait dispatch or trait impl method.
fn format_dispatch_declaration(
    ffi_name: &str,
    handle_c_name: &str,
    m: &Method,
    lib: &Library,
) -> String {
    let mut params = vec![format!("{handle_c_name} handle")];
    format_c_params(&m.params, lib, &mut params);

    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    let (ret_type, extra_params) = format_return_and_out_params(&m.ret, is_builder, lib);
    params.extend(extra_params);

    let params_str = params.join(", ");
    format!("{ret_type} {ffi_name}({params_str});")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute the C return type and any extra out-parameters for a method.
/// Returns `(c_return_type, extra_params_to_append)`.
fn format_return_and_out_params(
    ret: &Return,
    is_builder: bool,
    lib: &Library,
) -> (String, Vec<String>) {
    let error_c = blessed_c_type(lib, ffier_schema::Blessing::ErrorTrait);
    let result_c = || blessed_c_type(lib, ffier_schema::Blessing::Result);
    match ret {
        Return::Void => ("void".to_string(), vec![]),
        Return::Value(_) if is_builder => ("void".to_string(), vec![]),
        Return::Value(type_ref) => (lib.c_type_of(&type_ref.type_name).to_string(), vec![]),
        Return::Result {
            ok, c_convention, ..
        } => {
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
                        let c_type = lib.c_type_of(&ok_ref.type_name).to_string();
                        (
                            result_c(),
                            vec![format!("{}* result", c_type), format!("{error_c}* err_out")],
                        )
                    }
                    None => (result_c(), vec![format!("{error_c}* err_out")]),
                },
            }
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
