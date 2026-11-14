//! Emit safe wrappers and FFI declarations for free functions.

use std::fmt::Write;

use ffier_schema::{FreeFunction, Library};

use crate::calls::{ReturnPolicy, emit_call_body};
use crate::cfg::emit_cfg_attr;
use crate::ffi::{SymbolInfo, emit_extern_fns, extern_fn_from_free};
use crate::signature::{emit_doc_attrs, format_param_sig, lifetime_generics, plain_return_type};

pub(crate) fn emit_free_function(
    out: &mut String,
    f: &FreeFunction,
    lib: &Library,
    symbols: &mut Vec<SymbolInfo>,
) {
    // Extern declaration
    emit_extern_fns(out, &[extern_fn_from_free(f)], None, symbols);

    emit_cfg_attr(out, f.cfg.as_ref());
    emit_doc_attrs(out, &f.doc, "");

    let param_sigs: Vec<String> = f.params.iter().map(format_param_sig).collect();
    let params_str = param_sigs.join(", ");

    let ret_type = plain_return_type(&f.ret, lib);

    let lifetimes = lifetime_generics(&f.lifetimes);
    writeln!(
        out,
        "pub fn {}{lifetimes}({params_str}){ret_type} {{",
        f.name
    )
    .unwrap();

    emit_call_body(
        out,
        &f.ffi_name,
        None,
        &f.params,
        &f.ret,
        "    ",
        lib,
        ReturnPolicy::default(),
    );
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}
