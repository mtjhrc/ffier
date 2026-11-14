//! Fixed-layout value structs and their borrowed FFI conversions.

use std::collections::HashSet;
use std::fmt::Write;

use ffier_schema::Library;

use crate::cfg::emit_cfg_attr;
use crate::ffi_traits::emit_conversions;

/// Emit fixed-layout value structs, dependencies first so nested structs are
/// already declared when used as inline fields.
pub(crate) fn emit_value_structs(out: &mut String, lib: &Library) {
    fn visit<'a>(
        value: &'a ffier_schema::ValueStruct,
        lib: &'a Library,
        visiting: &mut HashSet<&'a str>,
        emitted: &mut HashSet<&'a str>,
        ordered: &mut Vec<&'a ffier_schema::ValueStruct>,
    ) {
        if emitted.contains(value.name.as_str()) || !visiting.insert(value.name.as_str()) {
            return;
        }
        for field in &value.fields {
            if lib.is_value_struct(&field.type_ref.type_name)
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

    let mut visiting = HashSet::new();
    let mut emitted = HashSet::new();
    let mut ordered = Vec::new();
    for value in &lib.value_structs {
        visit(value, lib, &mut visiting, &mut emitted, &mut ordered);
    }

    for value in ordered {
        emit_cfg_attr(out, value.cfg.as_ref());
        writeln!(out, "/// Fixed-layout value struct.").unwrap();
        writeln!(out, "#[repr(C)]").unwrap();
        writeln!(out, "#[derive(Debug, Clone, Copy)]").unwrap();
        writeln!(out, "pub struct {} {{", value.name).unwrap();
        for field in &value.fields {
            let ty = field.type_ref.to_rust_type();
            writeln!(out, "    pub {}: {},", field.name, ty).unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        let cfg = value.cfg.as_ref();
        let name = &value.name;
        emit_conversions(out, cfg, name, name, "self", "repr");
        emit_conversions(
            out,
            cfg,
            &format!("&{name}"),
            &format!("*const {name}"),
            "self",
            "unsafe { &*repr }",
        );
        emit_conversions(
            out,
            cfg,
            &format!("&mut {name}"),
            &format!("*mut {name}"),
            "self",
            "unsafe { &mut *repr }",
        );
        // Optional borrowed values are nullable pointers. Check for null
        // explicitly instead of relying on the null pointer optimization.
        emit_conversions(
            out,
            cfg,
            &format!("Option<&{name}>"),
            &format!("*const {name}"),
            "self.map_or(core::ptr::null(), |value| value)",
            "if repr.is_null() { None } else { Some(unsafe { &*repr }) }",
        );
        emit_conversions(
            out,
            cfg,
            &format!("Option<&mut {name}>"),
            &format!("*mut {name}"),
            "self.map_or(core::ptr::null_mut(), |value| value)",
            "if repr.is_null() { None } else { Some(unsafe { &mut *repr }) }",
        );
    }
}
