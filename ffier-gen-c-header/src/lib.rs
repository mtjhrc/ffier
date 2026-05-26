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
    MethodContext, ParamType, Receiver, Return, TraitImpl, TypeKind,
};

/// Generate a C header string from a library schema.
pub fn generate(lib: &Library, guard: &str) -> String {
    let mut out = String::new();
    let type_pfx = snake_to_pascal(&lib.prefix);
    let upper_pfx = format!("{}_", lib.prefix.to_ascii_uppercase());
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
    emit_trait_typedefs(&mut out, lib, &type_pfx);
    out.push('\n');

    // Shared types
    emit_shared_types(&mut out, &type_pfx, &upper_pfx);

    // Enum constant sections
    for en in &lib.enum_constants {
        emit_enum_section(&mut out, en);
    }

    // Error sections
    for err in &lib.errors {
        emit_error_section(&mut out, err, lib);
    }

    // Type sections (exportable methods + destroy)
    for ty in &lib.exported_types {
        emit_type_section(&mut out, ty, &type_pfx, lib);
    }

    // Implementable traits: vtable struct + dispatch functions
    for tr in &lib.traits {
        emit_vtable_section(&mut out, tr, &type_pfx, lib);
        emit_dispatch_section(&mut out, tr, &type_pfx, lib);
    }

    // Trait impl bridge functions
    for ti in &lib.trait_impls {
        emit_trait_impl_section(&mut out, ti, lib);
    }

    // Free functions
    if !lib.free_functions.is_empty() {
        emit_free_functions(&mut out, &lib.free_functions, &type_pfx, lib);
    }

    // Utility functions
    emit_utility_functions(&mut out, &fn_pfx, &type_pfx);

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

fn emit_shared_types(out: &mut String, type_pfx: &str, upper_pfx: &str) {
    let result_c = format!("{type_pfx}Result");
    let str_c = format!("{type_pfx}Str");
    let bytes_c = format!("{type_pfx}Bytes");
    let path_c = format!("{type_pfx}Path");
    let result_success = format!("{upper_pfx}RESULT_SUCCESS");
    let str_macro = format!("{upper_pfx}STR");
    let bytes_macro = format!("{upper_pfx}BYTES");

    // FIXME: These type definitions are hardcoded. The type registry knows
    // about str (kind: string), [u8] (kind: bytes), etc. but doesn't carry
    // their C struct layouts. These should be driven by the registry — e.g.
    // TypeKind::Struct { fields: [...] } or similar — so generators don't
    // need to hardcode ABI details.
    out.push_str(&format!("typedef uint64_t {result_c};\n"));
    out.push_str(&format!("#define {result_success} 0\n\n"));
    out.push_str("/* Caller must ensure data is valid UTF-8 */\n");
    out.push_str("typedef struct {\n");
    out.push_str("    const char* data;\n");
    out.push_str("    size_t len;\n");
    out.push_str(&format!("}} {str_c};\n\n"));
    out.push_str("/* Caller must ensure data is a valid UTF-8 path */\n");
    out.push_str(&format!("typedef {str_c} {path_c};\n\n"));
    out.push_str("typedef struct {\n");
    out.push_str("    const uint8_t* data;\n");
    out.push_str("    size_t len;\n");
    out.push_str(&format!("}} {bytes_c};\n\n"));
    out.push_str(&format!(
        "#define {str_macro}(s) (({str_c}){{ .data = (s), .len = strlen(s) }})\n"
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
    let vtable_handle_c = format!("{type_pfx}VtableHandle");
    let vtable_handle_macro = format!("{upper_pfx}VTABLE_HANDLE");
    out.push_str("/**\n");
    out.push_str(&format!(
        " * Stack-allocated temporary handle for passing vtable-based objects.\n"
    ));
    out.push_str(&format!(
        " * Only valid for the duration of the call — the callee borrows, not owns.\n"
    ));
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

fn emit_trait_typedefs(out: &mut String, lib: &Library, type_pfx: &str) {
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
        let vtable_name = format!("{type_pfx}Vtable{}", tr.name);
        trait_implementors
            .entry(tr.name.clone())
            .or_default()
            .push(vtable_name);
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

fn emit_vtable_section(out: &mut String, tr: &ImplementableTrait, type_pfx: &str, lib: &Library) {
    let vtable_name = format!("{type_pfx}{}Vtable", tr.name);

    emit_section_header(out, &format!("Vtable{}", tr.name));

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
            for p in &m.params {
                match &p.param_type {
                    ParamType::Regular(type_ref) => {
                        let c_type = lib.c_type_of(&type_ref.type_name);
                        params.push(format!("{} {}", c_type, p.name));
                    }
                    ParamType::Slice { c_params, .. } => {
                        for cp in c_params {
                            params.push(format!("{} {}", cp.c_type, cp.name));
                        }
                    }
                    ParamType::ImplTrait { trait_name, .. } => {
                        let c_type = lib.c_type_of(trait_name);
                        params.push(format!("{} {}", c_type, p.name));
                    }
                }
            }

            let ret_type = format_return_type_simple(&m.ret, type_pfx, lib);

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

fn emit_type_section(out: &mut String, ty: &ExportedType, type_pfx: &str, lib: &Library) {
    emit_section_header(out, &ty.name);

    let c_name = lib.c_type_of(&ty.name);

    for m in &ty.methods {
        let MethodContext::Exportable { ffi_name } = &m.context else {
            continue;
        };
        emit_doc_comment(out, &m.doc);
        let decl = format_c_declaration(ffi_name, c_name, m, ty.is_builder_type, type_pfx, lib);
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
        let type_pfx = snake_to_pascal(&lib.prefix);
        let decl = format_trait_method_declaration(ffi_name, &struct_c_name, m, &type_pfx, lib);
        out.push_str(&decl);
        out.push('\n');
    }
}

// ---------------------------------------------------------------------------
// Self-dispatch functions for implementable traits
// ---------------------------------------------------------------------------

fn emit_dispatch_section(out: &mut String, tr: &ImplementableTrait, type_pfx: &str, lib: &Library) {
    emit_section_header(out, &format!("{} (dispatch)", tr.name));

    let handle_c_name = lib.c_type_of(&tr.name);

    // Only emit dispatch for own methods (not supertrait methods)
    for m in tr.methods.iter().take(tr.own_method_count) {
        let MethodContext::Trait { ffi_name, .. } = &m.context else {
            continue;
        };
        emit_doc_comment(out, &m.doc);
        let decl = format_trait_method_declaration(ffi_name, handle_c_name, m, type_pfx, lib);
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
// Error handle functions
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

fn emit_free_functions(
    out: &mut String,
    functions: &[FreeFunction],
    type_pfx: &str,
    lib: &Library,
) {
    emit_section_header(out, "Free functions");

    for f in functions {
        emit_doc_comment(out, &f.doc);

        let mut params: Vec<String> = Vec::new();
        for p in &f.params {
            match &p.param_type {
                ParamType::Regular(type_ref) => {
                    let c_type = lib.c_type_of(&type_ref.type_name);
                    params.push(format!("{} {}", c_type, p.name));
                }
                ParamType::Slice { c_params: _, .. } => {
                    let str_c = format!("{type_pfx}Str");
                    params.push(format!("const {str_c}* {}", p.name));
                    params.push(format!("size_t {}_len", p.name));
                }
                ParamType::ImplTrait { trait_name, .. } => {
                    let c_type = lib.c_type_of(trait_name);
                    params.push(format!("{} {}", c_type, p.name));
                }
            }
        }

        let (ret_type, extra_params) = format_return_and_out_params(&f.ret, false, type_pfx, lib);
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

fn emit_utility_functions(out: &mut String, fn_pfx: &str, type_pfx: &str) {
    let result_c = format!("{type_pfx}Result");
    out.push_str(&format!(
        "{type_pfx}Str {fn_pfx}result_name({result_c} r);\n"
    ));
    out.push_str(&format!(
        "const char* {fn_pfx}result_name_cstr({result_c} r);\n"
    ));
}

// ---------------------------------------------------------------------------
// Declaration formatting
// ---------------------------------------------------------------------------

fn format_c_declaration(
    ffi_name: &str,
    handle_c_name: &str,
    m: &Method,
    is_builder_type: bool,
    type_pfx: &str,
    lib: &Library,
) -> String {
    let mut params: Vec<String> = Vec::new();

    // Handle param (self)
    match m.receiver {
        Receiver::None => {}
        Receiver::Value if is_builder_type => {
            // By-value self on builder type: pointer-to-handle so the bridge
            // can swap the handle after consuming the old value.
            params.push(format!("{handle_c_name}* handle"));
        }
        Receiver::Ref | Receiver::Mut | Receiver::Value => {
            params.push(format!("{handle_c_name} handle"));
        }
    }

    // Regular params
    for p in &m.params {
        match &p.param_type {
            ParamType::Regular(type_ref) => {
                let c_type = lib.c_type_of(&type_ref.type_name);
                params.push(format!("{} {}", c_type, p.name));
            }
            ParamType::Slice { c_params: _, .. } => {
                let str_c = format!("{type_pfx}Str");
                params.push(format!("const {str_c}* {}", p.name));
                params.push(format!("size_t {}_len", p.name));
            }
            ParamType::ImplTrait { trait_name, .. } => {
                let c_type = lib.c_type_of(trait_name);
                params.push(format!("{} {}", c_type, p.name));
            }
        }
    }

    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    let (ret_type, extra_params) = format_return_and_out_params(&m.ret, is_builder, type_pfx, lib);
    params.extend(extra_params);

    let params_str = params.join(", ");
    format!("{ret_type} {ffi_name}({params_str});")
}

fn format_trait_method_declaration(
    ffi_name: &str,
    handle_c_name: &str,
    m: &Method,
    type_pfx: &str,
    lib: &Library,
) -> String {
    let mut params = vec![format!("{handle_c_name} handle")];

    for p in &m.params {
        match &p.param_type {
            ParamType::Regular(type_ref) => {
                let c_type = lib.c_type_of(&type_ref.type_name);
                params.push(format!("{} {}", c_type, p.name));
            }
            ParamType::Slice { c_params: _, .. } => {
                let str_c = format!("{type_pfx}Str");
                params.push(format!("const {str_c}* {}", p.name));
                params.push(format!("size_t {}_len", p.name));
            }
            ParamType::ImplTrait { trait_name, .. } => {
                let c_type = lib.c_type_of(trait_name);
                params.push(format!("{} {}", c_type, p.name));
            }
        }
    }

    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    let (ret_type, extra_params) = format_return_and_out_params(&m.ret, is_builder, type_pfx, lib);
    params.extend(extra_params);

    let params_str = params.join(", ");
    format!("{ret_type} {ffi_name}({params_str});")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Simple C return type — used for vtable function pointer fields.
fn format_return_type_simple(ret: &Return, type_pfx: &str, lib: &Library) -> String {
    match ret {
        Return::Void => "void".to_string(),
        Return::Value(type_ref) => lib.c_type_of(&type_ref.type_name).to_string(),
        Return::Result { .. } => format!("{type_pfx}Result"),
    }
}

/// Compute the C return type and any extra out-parameters for a method.
/// Returns `(c_return_type, extra_params_to_append)`.
fn format_return_and_out_params(
    ret: &Return,
    is_builder: bool,
    type_pfx: &str,
    lib: &Library,
) -> (String, Vec<String>) {
    let error_c = find_error_c_type(lib);
    match ret {
        Return::Void => ("void".to_string(), vec![]),
        Return::Value(_) if is_builder => ("void".to_string(), vec![]),
        Return::Value(type_ref) => (lib.c_type_of(&type_ref.type_name).to_string(), vec![]),
        Return::Result { ok, .. } => match ok {
            None => {
                // Result<(), E>
                let result_c = format!("{type_pfx}Result");
                (result_c, vec![format!("{error_c}* err_out")])
            }
            Some(ok_ref) if is_builder => {
                // Builder Result<Self, E> — no ok out-param
                let result_c = format!("{type_pfx}Result");
                (result_c, vec![format!("{error_c}* err_out")])
            }
            Some(ok_ref) => {
                let is_handle = lib
                    .type_entry(&ok_ref.type_name)
                    .map(|e| e.kind == TypeKind::Handle)
                    .unwrap_or(false);
                if is_handle {
                    // Result<Handle, E> — return handle, NULL on error
                    (
                        lib.c_type_of(&ok_ref.type_name).to_string(),
                        vec![format!("{error_c}* err_out")],
                    )
                } else {
                    // Result<T, E> — out-param
                    let result_c = format!("{type_pfx}Result");
                    let c_type = lib.c_type_of(&ok_ref.type_name).to_string();
                    (
                        result_c,
                        vec![format!("{}* result", c_type), format!("{error_c}* err_out")],
                    )
                }
            }
        },
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

fn find_error_c_type(lib: &Library) -> String {
    let error_trait = lib
        .trait_by_pragma("error_trait")
        .expect("no trait with pragma \"error_trait\" found in schema");
    lib.c_type_of(&error_trait.name).to_string()
}

fn snake_to_pascal(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}
