//! Emit dynamic-lookup call shims and collect their symbol metadata.

use std::fmt::Write;

use ffier_schema::Cfg;

use crate::cfg::emit_cfg_attr;

use super::loading::symbol_to_variant;
use super::{ExternFn, SymbolInfo};

/// Emit a single dynlookup-binding shim using its cache in the ffi module.
/// Pushes symbol info into `symbols` for the enum/require generation.
pub(super) fn emit_dynlookup_shim(
    out: &mut String,
    f: &ExternFn,
    cfg: Option<&Cfg>,
    symbols: &mut Vec<SymbolInfo>,
) {
    let name = &f.name;
    let upper = name.to_ascii_uppercase();
    let params_str = f.params.join(", ");
    let ret = &f.ret;

    // Build the fn-pointer type
    let param_types: Vec<&str> = f
        .params
        .iter()
        .map(|p| p.split_once(": ").map(|(_, ty)| ty).unwrap_or(p))
        .collect();
    let fn_ptr_params = param_types.join(", ");
    let fn_ptr_type = format!("unsafe extern \"C\" fn({fn_ptr_params}){ret}");

    // Build the argument-forwarding list (just names)
    let arg_names: Vec<&str> = f
        .params
        .iter()
        .map(|p| p.split_once(": ").map(|(n, _)| n).unwrap_or(p))
        .collect();
    let args_str = arg_names.join(", ");

    // Shim function
    let vis = if f.public { "pub " } else { "" };
    emit_cfg_attr(out, cfg);
    writeln!(out, "#[cfg(not(feature = \"ffier-strong\"))]").unwrap();
    writeln!(out, "#[allow(non_snake_case)]").unwrap();
    if !f.public {
        writeln!(out, "#[allow(dead_code)]").unwrap();
    }
    if f.public {
        writeln!(out, "/// # Safety").unwrap();
        writeln!(out, "/// Uphold this function's C ABI contract, including pointer validity and handle ownership.").unwrap();
    }
    writeln!(out, "{vis}unsafe fn {name}({params_str}){ret} {{").unwrap();
    let variant = symbol_to_variant(name);
    writeln!(
        out,
        "    unsafe {{ (ffi::__ffier_symbol(ffi::Symbol::{variant}, &ffi::__symbols::{upper}))({args_str}) }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();

    symbols.push(SymbolInfo {
        c_name: name.clone(),
        upper: upper.clone(),
        fn_ptr_type,
        cfg: cfg.cloned(),
    });

    writeln!(out).unwrap();
}
