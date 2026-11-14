//! Extern declarations and symbol metadata shared by both linking modes.

use std::fmt::Write;

use ffier_schema::{Cfg, FreeFunction, Library, Method, Receiver};

use crate::abi::{push_extern_params, push_return_and_out_params};
use crate::cfg;

mod dynlookup;
mod loading;

use dynlookup::emit_dynlookup_shim;
pub(crate) use loading::emit_symbol_api;

/// A single extern "C" function declaration to emit.
pub(crate) struct ExternFn {
    /// Function name (e.g. `"ft_widget_new"`).
    name: String,
    /// Typed parameters (e.g. `["handle: *mut core::ffi::c_void", "x: i32"]`).
    params: Vec<String>,
    /// Return type string including ` -> ...`, or empty for void.
    ret: String,
    /// Whether to emit as `pub fn` (true) or `fn` (false) in extern blocks.
    public: bool,
    /// `#[cfg(...)]` predicate. `None` = unconditional.
    cfg: Option<ffier_schema::Cfg>,
}

/// Build a `pub` `ExternFn` from a name, param list, and return string.
pub(crate) fn extern_fn(name: &str, params: Vec<String>, ret: String) -> ExternFn {
    ExternFn {
        name: name.to_string(),
        params,
        ret,
        public: true,
        cfg: None,
    }
}

/// Build a `pub` `ExternFn` with an optional cfg predicate.
fn extern_fn_cfg(
    name: &str,
    params: Vec<String>,
    ret: String,
    cfg: Option<ffier_schema::Cfg>,
) -> ExternFn {
    ExternFn {
        name: name.to_string(),
        params,
        ret,
        public: true,
        cfg,
    }
}

/// Build a non-`pub` `ExternFn`.
pub(crate) fn extern_fn_private(name: &str, params: Vec<String>, ret: String) -> ExternFn {
    ExternFn {
        name: name.to_string(),
        params,
        ret,
        public: false,
        cfg: None,
    }
}

/// Build an `ExternFn` from a method's signature.
pub(crate) fn extern_fn_from_method(ffi_name: &str, m: &Method, lib: &Library) -> ExternFn {
    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    let mut params = Vec::new();
    match m.receiver {
        Receiver::None => {}
        _ => {
            params.push("handle: *mut core::ffi::c_void".to_string());
        }
    }
    push_extern_params(&m.params, &mut params);
    let ret = push_return_and_out_params(&m.ret, &mut params, is_builder);
    extern_fn_cfg(ffi_name, params, ret, m.cfg.clone())
}

/// Build an `ExternFn` from a dispatch method signature.
pub(crate) fn extern_fn_from_dispatch(ffi_name: &str, m: &Method) -> ExternFn {
    let mut params = vec!["handle: *mut core::ffi::c_void".to_string()];
    push_extern_params(&m.params, &mut params);
    let ret = push_return_and_out_params(&m.ret, &mut params, false);
    extern_fn_cfg(ffi_name, params, ret, m.cfg.clone())
}

/// Build an `ExternFn` from a free function.
pub(crate) fn extern_fn_from_free(f: &FreeFunction) -> ExternFn {
    let mut params = Vec::new();
    push_extern_params(&f.params, &mut params);
    let ret = push_return_and_out_params(&f.ret, &mut params, false);
    extern_fn_cfg(&f.ffi_name, params, ret, f.cfg.clone())
}

/// Emit complementary `ffier-strong` declarations and dynlookup shims with shared names.
pub(crate) fn emit_extern_fns(
    out: &mut String,
    fns: &[ExternFn],
    cfg: Option<&ffier_schema::Cfg>,
    symbols: &mut Vec<SymbolInfo>,
) {
    if fns.is_empty() {
        return;
    }
    writeln!(out, "#[cfg(feature = \"ffier-strong\")]").unwrap();
    if let Some(pred) = cfg {
        writeln!(out, "#[cfg({pred})]").unwrap();
    }
    writeln!(out, "unsafe extern \"C\" {{").unwrap();
    for f in fns {
        let vis = if f.public { "pub " } else { "" };
        let params_str = f.params.join(", ");
        if let Some(cfg) = &f.cfg {
            writeln!(out, "    #[cfg({cfg})]").unwrap();
        }
        writeln!(out, "    {vis}fn {}({params_str}){};", f.name, f.ret).unwrap();
    }
    writeln!(out, "}}\n").unwrap();
    for f in fns {
        let effective_cfg = cfg::combine(cfg, f.cfg.as_ref());
        emit_dynlookup_shim(out, f, effective_cfg.as_ref(), symbols);
    }
}

/// Metadata collected once per FFI symbol for both linking modes.
pub(crate) struct SymbolInfo {
    /// C symbol name (e.g. `"ft_widget_new"`)
    c_name: String,
    /// Rust OnceLock static name (e.g. `"FT_WIDGET_NEW"`)
    upper: String,
    /// fn pointer type string (e.g. `"unsafe extern \"C\" fn() -> *mut c_void"`)
    fn_ptr_type: String,
    /// Combined availability of the owning item and function.
    cfg: Option<Cfg>,
}
