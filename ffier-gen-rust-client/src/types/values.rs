//! Fixed-layout value structs and their borrowed FFI conversions.

use std::collections::HashSet;
use std::fmt::Write;

use ffier_schema::Library;

use crate::cfg::emit_cfg_attr;

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

        emit_cfg_attr(out, value.cfg.as_ref());
        writeln!(out, "impl FfiType for {} {{", value.name).unwrap();
        writeln!(out, "    type CRepr = Self;").unwrap();
        writeln!(
            out,
            "    const C_TYPE_NAME: &'static str = \"{}\";",
            value.name
        )
        .unwrap();
        writeln!(out, "    fn into_c(self) -> Self {{ self }}").unwrap();
        writeln!(out, "    unsafe fn from_c(repr: Self) -> Self {{ repr }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        for (ptr, name) in [
            ("*const", format!("&{}", value.name)),
            ("*mut", format!("&mut {}", value.name)),
        ] {
            emit_cfg_attr(out, value.cfg.as_ref());
            writeln!(out, "impl FfiType for {name} {{").unwrap();
            writeln!(out, "    type CRepr = {ptr} {};", value.name).unwrap();
            writeln!(
                out,
                "    const C_TYPE_NAME: &'static str = \"{}\";",
                value.name
            )
            .unwrap();
            writeln!(out, "    fn into_c(self) -> Self::CRepr {{ self }}").unwrap();
            if ptr == "*const" {
                writeln!(
                    out,
                    "    unsafe fn from_c(repr: Self::CRepr) -> Self {{ unsafe {{ &*repr }} }}"
                )
                .unwrap();
            } else {
                writeln!(
                    out,
                    "    unsafe fn from_c(repr: Self::CRepr) -> Self {{ unsafe {{ &mut *repr }} }}"
                )
                .unwrap();
            }
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }

        // Optional borrowed values are nullable pointers. Check for null
        // explicitly instead of relying on the null pointer optimization.
        for (ptr, name, null) in [
            ("*const", format!("Option<&{}>", value.name), "null"),
            ("*mut", format!("Option<&mut {}>", value.name), "null_mut"),
        ] {
            emit_cfg_attr(out, value.cfg.as_ref());
            writeln!(out, "impl FfiType for {name} {{").unwrap();
            writeln!(out, "    type CRepr = {ptr} {};", value.name).unwrap();
            writeln!(
                out,
                "    const C_TYPE_NAME: &'static str = \"{}\";",
                value.name
            )
            .unwrap();
            writeln!(
                out,
                "    fn into_c(self) -> Self::CRepr {{ self.map_or(core::ptr::{null}(), |value| value) }}"
            )
            .unwrap();
            if ptr == "*const" {
                writeln!(
                    out,
                    "    unsafe fn from_c(repr: Self::CRepr) -> Self {{ if repr.is_null() {{ None }} else {{ Some(unsafe {{ &*repr }}) }} }}"
                )
                .unwrap();
            } else {
                writeln!(
                    out,
                    "    unsafe fn from_c(repr: Self::CRepr) -> Self {{ if repr.is_null() {{ None }} else {{ Some(unsafe {{ &mut *repr }}) }} }}"
                )
                .unwrap();
            }
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }
}
