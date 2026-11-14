//! Enum and bitflags definitions and FFI conversions.

use std::fmt::Write;

use ffier_schema::{EnumType, Library, TypeKind};

use crate::cfg::emit_cfg_attr;

pub(crate) fn emit_enum_type(out: &mut String, en: &EnumType, lib: &Library) {
    let entry = lib.type_entry(&en.name).unwrap();
    let repr = match &entry.kind {
        TypeKind::Enum { alias_of } => alias_of.as_str(),
        _ => panic!("enum type must have TypeKind::Enum"),
    };

    // Enum definition
    emit_cfg_attr(out, en.cfg.as_ref());
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]").unwrap();
    writeln!(out, "#[repr({repr})]").unwrap();
    writeln!(out, "pub enum {} {{", en.name).unwrap();
    for v in &en.variants {
        writeln!(out, "    {} = {},", v.name, v.value).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiType impl
    emit_cfg_attr(out, en.cfg.as_ref());
    writeln!(out, "impl FfiType for {} {{", en.name).unwrap();
    writeln!(out, "    type CRepr = {repr};").unwrap();
    writeln!(
        out,
        "    const C_TYPE_NAME: &'static str = \"{}\";",
        en.name
    )
    .unwrap();
    writeln!(out, "    fn into_c(self) -> {repr} {{ self as {repr} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: {repr}) -> Self {{").unwrap();
    writeln!(out, "        match repr {{").unwrap();
    for v in &en.variants {
        writeln!(out, "            {} => Self::{},", v.value, v.name).unwrap();
    }
    writeln!(
        out,
        "            unknown => panic!(\"invalid {} discriminant: {{}}\", unknown),",
        en.name
    )
    .unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ===========================================================================
// Bitflags type generation
// ===========================================================================

pub(crate) fn emit_bitflags_type(out: &mut String, bf: &EnumType, lib: &Library) {
    let entry = lib.type_entry(&bf.name).unwrap();
    let repr = match &entry.kind {
        TypeKind::Bitflags { alias_of } => alias_of.as_str(),
        _ => panic!("bitflags type must have TypeKind::Bitflags"),
    };

    // bitflags! invocation
    emit_cfg_attr(out, bf.cfg.as_ref());
    writeln!(out, "bitflags::bitflags! {{").unwrap();
    writeln!(out, "    #[repr(transparent)]").unwrap();
    writeln!(out, "    #[derive(Debug, Clone, Copy, PartialEq, Eq)]").unwrap();
    writeln!(out, "    pub struct {}: {repr} {{", bf.name).unwrap();
    for v in &bf.variants {
        writeln!(out, "        const {} = {};", v.name, v.value).unwrap();
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiType impl
    emit_cfg_attr(out, bf.cfg.as_ref());
    writeln!(out, "impl FfiType for {} {{", bf.name).unwrap();
    writeln!(out, "    type CRepr = {repr};").unwrap();
    writeln!(
        out,
        "    const C_TYPE_NAME: &'static str = \"{}\";",
        bf.name
    )
    .unwrap();
    writeln!(out, "    fn into_c(self) -> {repr} {{ self.bits() }}").unwrap();
    writeln!(
        out,
        "    unsafe fn from_c(repr: {repr}) -> Self {{ Self::from_bits_retain(repr) }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}
