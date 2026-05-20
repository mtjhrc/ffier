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
    ErrorType, ExportedType, ImplementableTrait, Library, Method, MethodContext, ParamType,
    Receiver, Return, TraitImpl, TypeKind,
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

    // Handle typedefs — errors first (convention from existing generator)
    for err in &lib.errors {
        let c_name = &lib.type_registry[&err.name].c_type;
        out.push_str(&format!("typedef void* {};\n", c_name));
    }
    for ty in &lib.exported_types {
        let c_name = &lib.type_registry[&ty.name].c_type;
        out.push_str(&format!("typedef void* {};\n", c_name));
    }

    // Trait typedefs with implementor lists
    emit_trait_typedefs(&mut out, lib, &type_pfx);
    out.push('\n');

    // Shared types
    emit_shared_types(&mut out, &type_pfx, &upper_pfx);

    // Error sections
    for err in &lib.errors {
        emit_error_section(&mut out, err, lib);
    }

    // Type sections (exportable methods + destroy)
    for ty in &lib.exported_types {
        emit_type_section(&mut out, ty, &type_pfx, &fn_pfx, lib);
    }

    // Implementable traits: vtable struct + dispatch functions
    for tr in &lib.traits {
        emit_vtable_section(&mut out, tr, &type_pfx, lib);
        emit_dispatch_section(&mut out, tr, &fn_pfx, &type_pfx, lib);
    }

    // Trait impl bridge functions
    for ti in &lib.trait_impls {
        emit_trait_impl_section(&mut out, ti, &fn_pfx, lib);
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
    let error_c = format!("{type_pfx}Error");
    let str_c = format!("{type_pfx}Str");
    let bytes_c = format!("{type_pfx}Bytes");
    let path_c = format!("{type_pfx}Path");
    let result_success = format!("{upper_pfx}RESULT_SUCCESS");
    let str_macro = format!("{upper_pfx}STR");
    let bytes_macro = format!("{upper_pfx}BYTES");

    out.push_str(&format!("typedef uint64_t {result_c};\n"));
    out.push_str(&format!("#define {result_success} 0\n\n"));
    out.push_str(
        "/* Opaque error handle — pass to *_error_message() for details, free with *_error_destroy() */\n",
    );
    out.push_str(&format!("typedef void* {error_c};\n\n"));
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
        let c_name = format!("{type_pfx}{trait_name}");
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
                        let c_type = lib.c_type(type_ref);
                        params.push(format!("{} {}", c_type, p.name));
                    }
                    ParamType::Slice { c_params, .. } => {
                        for cp in c_params {
                            params.push(format!("{} {}", cp.c_type, cp.name));
                        }
                    }
                    ParamType::ImplTrait { trait_name, .. } => {
                        let c_type = lib.trait_c_type(trait_name);
                        params.push(format!("{} {}", c_type, p.name));
                    }
                }
            }

            let ret_type = format_return_type(&m.ret, type_pfx, lib);

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

fn emit_type_section(
    out: &mut String,
    ty: &ExportedType,
    type_pfx: &str,
    fn_pfx: &str,
    lib: &Library,
) {
    emit_section_header(out, &ty.name);

    let c_name = &lib.type_registry[&ty.name].c_type;

    for m in &ty.methods {
        let MethodContext::Exportable {
            ffi_name,
            is_builder,
        } = &m.context
        else {
            continue;
        };
        emit_doc_comment(out, &m.doc);
        let decl = format_c_declaration(
            ffi_name,
            c_name,
            m,
            *is_builder,
            ty.is_builder_type,
            type_pfx,
            lib,
        );
        out.push_str(&decl);
        out.push('\n');
    }

    // Destroy function
    let type_snake = camel_to_snake(&ty.name);
    out.push_str(&format!(
        "void {fn_pfx}{type_snake}_destroy({} handle);\n",
        c_name
    ));
}

// ---------------------------------------------------------------------------
// Trait impl bridge functions
// ---------------------------------------------------------------------------

fn emit_trait_impl_section(out: &mut String, ti: &TraitImpl, fn_pfx: &str, lib: &Library) {
    let _trait_snake = camel_to_snake(&ti.trait_name);
    let struct_snake = camel_to_snake(&ti.struct_name);
    let struct_c_name = find_type_c_name(lib, &ti.struct_name);

    for m in &ti.methods {
        let MethodContext::Trait { .. } = &m.context else {
            continue;
        };
        let ffi_name = format!("{fn_pfx}{struct_snake}_{}", m.name);
        let type_pfx = snake_to_pascal(&lib.prefix);
        let decl = format_trait_method_declaration(&ffi_name, &struct_c_name, m, &type_pfx, lib);
        out.push_str(&decl);
        out.push('\n');
    }
}

// ---------------------------------------------------------------------------
// Self-dispatch functions for implementable traits
// ---------------------------------------------------------------------------

fn emit_dispatch_section(
    out: &mut String,
    tr: &ImplementableTrait,
    fn_pfx: &str,
    type_pfx: &str,
    lib: &Library,
) {
    emit_section_header(out, &format!("{} (dispatch)", tr.name));

    // Only emit dispatch for own methods (not supertrait methods)
    for m in tr.methods.iter().take(tr.own_method_count) {
        let MethodContext::Trait { .. } = &m.context else {
            continue;
        };
        let trait_snake = camel_to_snake(&tr.name);
        let ffi_name = format!("{fn_pfx}{trait_snake}_{}", m.name);

        let mut params = vec![format!("void* handle")];
        for p in &m.params {
            match &p.param_type {
                ParamType::Regular(type_ref) => {
                    let c_type = lib.c_type(type_ref);
                    params.push(format!("{} {}", c_type, p.name));
                }
                ParamType::Slice { c_params, .. } => {
                    for cp in c_params {
                        params.push(format!("{} {}", cp.c_type, cp.name));
                    }
                }
                ParamType::ImplTrait { trait_name, .. } => {
                    let c_type = lib.trait_c_type(trait_name);
                    params.push(format!("{} {}", c_type, p.name));
                }
            }
        }

        let ret_type = format_return_type(&m.ret, type_pfx, lib);

        let params_str = params.join(", ");
        out.push_str(&format!("{ret_type} {ffi_name}({params_str});\n"));
    }

    // Destroy dispatch
    let trait_snake = camel_to_snake(&tr.name);
    out.push_str(&format!(
        "void {fn_pfx}{trait_snake}_destroy(void* handle);\n"
    ));
}

// ---------------------------------------------------------------------------
// Error handle functions
// ---------------------------------------------------------------------------

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
    is_builder: bool,
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
                let c_type = lib.c_type(type_ref);
                params.push(format!("{} {}", c_type, p.name));
            }
            ParamType::Slice { c_params, .. } => {
                let str_c = format!("{type_pfx}Str");
                params.push(format!("const {str_c}* {}", p.name));
                params.push(format!("uintptr_t {}_len", p.name));
            }
            ParamType::ImplTrait { trait_name, .. } => {
                let c_type = lib.trait_c_type(trait_name);
                params.push(format!("{} {}", c_type, p.name));
            }
        }
    }

    // Return type handling
    let error_c = format!("{type_pfx}Error");
    let (ret_type, extra_params) = match &m.ret {
        Return::Void => ("void".to_string(), vec![]),
        Return::Value(type_ref) => {
            if is_builder {
                ("void".to_string(), vec![])
            } else {
                (lib.c_type(type_ref).to_string(), vec![])
            }
        }
        Return::Result { ok, .. } => {
            match ok {
                None => {
                    // Result<(), E>
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
                            lib.c_type(ok_ref).to_string(),
                            vec![format!("{error_c}* err_out")],
                        )
                    } else {
                        // Result<T, E> — out-param
                        let result_c = format!("{type_pfx}Result");
                        let c_type = lib.c_type(ok_ref).to_string();
                        (
                            result_c,
                            vec![format!("{}* result", c_type), format!("{error_c}* err_out")],
                        )
                    }
                }
            }
        }
    };

    params.extend(extra_params);

    let params_str = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };

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
                let c_type = lib.c_type(type_ref);
                params.push(format!("{} {}", c_type, p.name));
            }
            ParamType::Slice { c_params, .. } => {
                let str_c = format!("{type_pfx}Str");
                params.push(format!("const {str_c}* {}", p.name));
                params.push(format!("uintptr_t {}_len", p.name));
            }
            ParamType::ImplTrait { trait_name, .. } => {
                let c_type = lib.trait_c_type(trait_name);
                params.push(format!("{} {}", c_type, p.name));
            }
        }
    }

    let ret_type = format_return_type(&m.ret, type_pfx, lib);

    let params_str = params.join(", ");
    format!("{ret_type} {ffi_name}({params_str});")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format the C return type string for a method return.
fn format_return_type(ret: &Return, type_pfx: &str, lib: &Library) -> String {
    match ret {
        Return::Void => "void".to_string(),
        Return::Value(type_ref) => lib.c_type(type_ref).to_string(),
        Return::Result { .. } => format!("{type_pfx}Result"),
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
    lib.type_registry
        .get(name)
        .map(|e| e.c_type.clone())
        .unwrap_or_else(|| {
            let type_pfx = snake_to_pascal(&lib.prefix);
            format!("{type_pfx}{name}")
        })
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

fn camel_to_snake(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}
