//! Emit safe wrappers and FFI declarations for free functions.

use std::fmt::Write;

use ffier_schema::{FreeFunction, Library, Return};

use crate::abi::optional_value_out_type;
use crate::calls::{build_ffi_param_args, emit_param_pre_bindings};
use crate::cfg::emit_cfg_attr;
use crate::error::find_error_dispatch_fns;
use crate::ffi::{SymbolInfo, emit_extern_fns, extern_fn_from_free};
use crate::signature::{format_param_sig, lifetime_generics};

pub(crate) fn emit_free_function(
    out: &mut String,
    f: &FreeFunction,
    lib: &Library,
    symbols: &mut Vec<SymbolInfo>,
) {
    // Extern declaration
    emit_extern_fns(out, &[extern_fn_from_free(f)], None, symbols);

    emit_cfg_attr(out, f.cfg.as_ref());
    for doc in &f.doc {
        let escaped = doc.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(out, "#[doc = \"{escaped}\"]").unwrap();
    }

    let param_sigs: Vec<String> = f.params.iter().map(format_param_sig).collect();
    let params_str = param_sigs.join(", ");

    let ret_type = match &f.ret {
        Return::Void => String::new(),
        Return::ObjectArray { element } => {
            format!(" -> ForeignSlice<{}>", element.type_name)
        }
        Return::Value(tr) => format!(" -> {}", tr.to_rust_type()),
        Return::OptionalValue(tr) => format!(" -> Option<{}>", tr.to_rust_type()),
        Return::Result { ok, err_type, .. } => {
            let ok_str = match ok {
                Some(tr) => tr.to_rust_type(),
                None => "()".to_string(),
            };
            format!(" -> Result<{ok_str}, {err_type}>")
        }
    };

    let lifetimes = lifetime_generics(&f.lifetimes);
    writeln!(
        out,
        "pub fn {}{lifetimes}({params_str}){ret_type} {{",
        f.name
    )
    .unwrap();

    emit_param_pre_bindings(out, &f.params, "    ");
    let ffi_args = build_ffi_param_args(&f.params, lib);
    let args_str = ffi_args.join(", ");
    let sep = if args_str.is_empty() { "" } else { ", " };
    match &f.ret {
        Return::Void => {
            writeln!(out, "    unsafe {{ {}({args_str}) }}", f.ffi_name).unwrap();
        }
        Return::ObjectArray { .. } => {
            writeln!(
                out,
                "    ForeignSlice::from_raw(unsafe {{ {}({args_str}) }})",
                f.ffi_name
            )
            .unwrap();
        }
        Return::OptionalValue(tr) => {
            let ty_static = optional_value_out_type(tr);
            let mut bare = tr.clone();
            bare.optional = false;
            let ty = bare.to_rust_type();
            writeln!(
                out,
                "    let mut __out = std::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();"
            )
            .unwrap();
            writeln!(
                out,
                "    let __is_some = unsafe {{ {}({args_str}{sep}__out.as_mut_ptr()) }};",
                f.ffi_name
            )
            .unwrap();
            writeln!(out, "    if __is_some {{").unwrap();
            writeln!(
                out,
                "        Some(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }})"
            )
            .unwrap();
            writeln!(out, "    }} else {{").unwrap();
            writeln!(out, "        None").unwrap();
            writeln!(out, "    }}").unwrap();
        }
        Return::Value(tr) => {
            writeln!(
                out,
                "    let __raw = unsafe {{ {}({args_str}) }};",
                f.ffi_name
            )
            .unwrap();
            let ty = tr.to_rust_type();
            writeln!(out, "    unsafe {{ <{ty} as FfiType>::from_c(__raw) }}").unwrap();
        }
        Return::Result {
            ok: None, err_type, ..
        } => {
            writeln!(
                out,
                "    let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(
                out,
                "    let __r = unsafe {{ {}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};",
                f.ffi_name
            )
            .unwrap();
            writeln!(
                out,
                "    if __r == 0 {{ Ok(()) }} else {{ Err(unsafe {{ {err_type}::from_ffi(__r, __err) }}) }}"
            )
            .unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            c_convention,
        } => {
            if *c_convention == ffier_schema::CResultConvention::OptionalValueOutParam {
                // Presence flag plus a separate value out-param.
                let ty_static = optional_value_out_type(ok_tr);
                let mut bare = ok_tr.clone();
                bare.optional = false;
                let ty = bare.to_rust_type();
                writeln!(out, "    let mut __result_is_some = false;").unwrap();
                writeln!(
                    out,
                    "    let mut __out = std::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();"
                )
                .unwrap();
                writeln!(
                    out,
                    "    let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
                )
                .unwrap();
                writeln!(
                    out,
                    "    let __r = unsafe {{ {}({args_str}{sep}&mut __result_is_some as *mut bool, __out.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) }};",
                    f.ffi_name
                )
                .unwrap();
                writeln!(out, "    if __r == 0 {{").unwrap();
                writeln!(out, "        if __result_is_some {{").unwrap();
                writeln!(
                    out,
                    "            Ok(Some(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }}))"
                )
                .unwrap();
                writeln!(out, "        }} else {{").unwrap();
                writeln!(out, "            Ok(None)").unwrap();
                writeln!(out, "        }}").unwrap();
                writeln!(out, "    }} else {{").unwrap();
                writeln!(
                    out,
                    "        Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                )
                .unwrap();
                writeln!(out, "    }}").unwrap();
            } else if *c_convention == ffier_schema::CResultConvention::HandleOrNull {
                writeln!(
                    out,
                    "    let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
                )
                .unwrap();
                writeln!(
                    out,
                    "    let __raw = unsafe {{ {}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};",
                    f.ffi_name
                )
                .unwrap();
                let (error_result_fn, _error_destroy_fn) = find_error_dispatch_fns(lib);
                writeln!(out, "    if !__raw.is_null() {{").unwrap();
                let ty = ok_tr.to_rust_type();
                writeln!(
                    out,
                    "        Ok(unsafe {{ <{ty} as FfiType>::from_c(__raw) }})"
                )
                .unwrap();
                writeln!(out, "    }} else {{").unwrap();
                writeln!(
                    out,
                    "        let __r = unsafe {{ {error_result_fn}(__err) }};"
                )
                .unwrap();
                writeln!(
                    out,
                    "        Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                )
                .unwrap();
                writeln!(out, "    }}").unwrap();
            } else {
                writeln!(out, "    let mut __out = std::mem::MaybeUninit::uninit();").unwrap();
                writeln!(
                    out,
                    "    let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
                )
                .unwrap();
                writeln!(
                    out,
                    "    let __r = unsafe {{ {}({args_str}{sep}__out.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) }};",
                    f.ffi_name
                )
                .unwrap();
                writeln!(out, "    if __r == 0 {{").unwrap();
                let ty = ok_tr.to_rust_type();
                writeln!(
                    out,
                    "        Ok(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }})"
                )
                .unwrap();
                writeln!(out, "    }} else {{").unwrap();
                writeln!(
                    out,
                    "        Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                )
                .unwrap();
                writeln!(out, "    }}").unwrap();
            }
        }
    }

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}
