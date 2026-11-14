//! Enum and bitflags definitions and FFI conversions.

use std::fmt::Write;

use ffier_schema::{EnumType, Library, TypeKind};

use crate::cfg::emit_cfg_attr;
use crate::ffi_traits::emit_conversions;

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

    let mut from_c = String::from("match repr { ");
    for v in &en.variants {
        write!(from_c, "{} => Self::{}, ", v.value, v.name).unwrap();
    }
    write!(
        from_c,
        "unknown => panic!(\"invalid {} discriminant: {{}}\", unknown) }}",
        en.name
    )
    .unwrap();
    emit_conversions(
        out,
        en.cfg.as_ref(),
        &en.name,
        repr,
        &format!("self as {repr}"),
        &from_c,
    );
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

    emit_conversions(
        out,
        bf.cfg.as_ref(),
        &bf.name,
        repr,
        "self.bits()",
        "Self::from_bits_retain(repr)",
    );
}
