//! Marshal safe wrapper arguments and results around FFI calls.

use std::fmt::Write;

use ffier_schema::{Library, ParamType, Return};

use crate::abi::optional_value_out_type;
use crate::error::find_error_dispatch_fns;

/// Build FFI call arguments from method params. Each param is converted
/// via `FfiType::into_c`, `as_handle()`, `__into_raw_handle()`, or pre-bound
/// slice reference. For borrowed handle params (`&Handle`), uses `as_handle()`
/// instead of `into_c()` to avoid lifetime escape.
pub(crate) fn build_ffi_param_args(params: &[ffier_schema::Param], lib: &Library) -> Vec<String> {
    let mut args = Vec::new();
    for p in params {
        match &p.param_type {
            ParamType::Regular(tr) => {
                // For borrowed references to handle types, use as_handle()
                // to avoid lifetime escape from into_c().
                let is_borrowed_handle = tr.ref_kind != ffier_schema::RefKind::None
                    && lib
                        .type_entry(&tr.type_name)
                        .is_some_and(|e| e.kind.is_handle());
                if is_borrowed_handle {
                    args.push(format!("FfiHandle::as_handle({})", p.name));
                } else {
                    let ty = tr.to_rust_type();
                    args.push(format!("<{ty} as FfiType>::into_c({})", p.name));
                }
            }
            ParamType::Slice { element, .. } => {
                if element.ref_kind != ffier_schema::RefKind::None {
                    // Ref elements (str, handles) were converted into a Vec
                    // by emit_param_pre_bindings — use the converted Vec.
                    args.push(format!("__ffi_{}.as_ptr()", p.name));
                    args.push(format!("__ffi_{}.len()", p.name));
                } else {
                    // Non-ref elements (primitives) are layout-compatible,
                    // pass the slice data directly.
                    args.push(format!("{}.as_ptr()", p.name));
                    args.push(format!("{}.len()", p.name));
                }
            }
            ParamType::ImplTrait { ref_kind, .. } => {
                args.push(match ref_kind {
                    ffier_schema::RefKind::None => {
                        format!("{}.__into_callback_handle().into_raw()", p.name)
                    }
                    _ => format!("__ffi_{}.as_raw()", p.name),
                });
            }
        }
    }
    args
}

/// Emit borrowed callback guards and ref-element slice conversions.
pub(crate) fn emit_param_pre_bindings(
    out: &mut String,
    params: &[ffier_schema::Param],
    indent: &str,
) {
    for p in params {
        if let ParamType::ImplTrait { ref_kind, .. } = &p.param_type {
            let method = match ref_kind {
                ffier_schema::RefKind::None => continue,
                ffier_schema::RefKind::Shared => "__borrow_callback_handle",
                ffier_schema::RefKind::Mut => "__borrow_callback_handle_mut",
            };
            writeln!(out, "{indent}let __ffi_{} = {}.{method}();", p.name, p.name).unwrap();
        }
        if let ParamType::Slice { element, .. } = &p.param_type
            && element.ref_kind != ffier_schema::RefKind::None
        {
            let elem_ty = element.to_rust_type();
            writeln!(
                out,
                "{indent}let __ffi_{}: Vec<<{elem_ty} as FfiType>::CRepr> = {}.iter().map(|s| <{elem_ty} as FfiType>::into_c(*s)).collect();",
                p.name, p.name,
            )
            .unwrap();
        }
    }
}

/// Emit an FFI call and handle the return value conversion.
pub(crate) fn emit_ffi_call_return(
    out: &mut String,
    ffi_name: &str,
    args_str: &str,
    ret: &Return,
    indent: &str,
    lib: &Library,
) {
    let sep = if args_str.is_empty() { "" } else { ", " };
    match ret {
        Return::Void => {
            writeln!(out, "{indent}unsafe {{ {ffi_name}({args_str}) }}").unwrap();
        }
        Return::ObjectArray { .. } => {
            writeln!(
                out,
                "{indent}ForeignSlice::from_raw(unsafe {{ {ffi_name}({args_str}) }})"
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
                "{indent}let mut __out = std::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();"
            )
            .unwrap();
            writeln!(
                out,
                "{indent}let __is_some = unsafe {{ {ffi_name}({args_str}{sep}__out.as_mut_ptr()) }};"
            )
            .unwrap();
            writeln!(out, "{indent}if __is_some {{").unwrap();
            writeln!(
                out,
                "{indent}    Some(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }})"
            )
            .unwrap();
            writeln!(out, "{indent}}} else {{").unwrap();
            writeln!(out, "{indent}    None").unwrap();
            writeln!(out, "{indent}}}").unwrap();
        }
        Return::Value(tr) => {
            writeln!(
                out,
                "{indent}let __raw = unsafe {{ {ffi_name}({args_str}) }};"
            )
            .unwrap();
            let ty = tr.to_rust_type();
            writeln!(out, "{indent}unsafe {{ <{ty} as FfiType>::from_c(__raw) }}").unwrap();
        }
        Return::Result {
            ok: None, err_type, ..
        } => {
            writeln!(
                out,
                "{indent}let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(out, "{indent}let __r = unsafe {{ {ffi_name}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
            writeln!(
                out,
                "{indent}if __r == 0 {{ Ok(()) }} else {{ Err(unsafe {{ {err_type}::from_ffi(__r, __err) }}) }}"
            )
            .unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            c_convention,
        } => {
            use ffier_schema::CResultConvention;
            writeln!(
                out,
                "{indent}let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            match c_convention {
                CResultConvention::HandleOrNull => {
                    let (error_result_fn, _) = find_error_dispatch_fns(lib);
                    writeln!(out, "{indent}let __raw = unsafe {{ {ffi_name}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
                    writeln!(out, "{indent}if !__raw.is_null() {{").unwrap();
                    let ty = ok_tr.to_rust_type();
                    writeln!(
                        out,
                        "{indent}    Ok(unsafe {{ <{ty} as FfiType>::from_c(__raw) }})"
                    )
                    .unwrap();
                    writeln!(out, "{indent}}} else {{").unwrap();
                    writeln!(
                        out,
                        "{indent}    let __r = unsafe {{ {error_result_fn}(__err) }};"
                    )
                    .unwrap();
                    writeln!(
                        out,
                        "{indent}    Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                    )
                    .unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                }
                CResultConvention::OutParam => {
                    let ty_static = ok_tr.to_rust_type_static();
                    writeln!(out, "{indent}let mut __result = core::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();").unwrap();
                    writeln!(out, "{indent}let __r = unsafe {{ {ffi_name}({args_str}{sep}__result.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
                    writeln!(out, "{indent}if __r == 0 {{").unwrap();
                    let ty = ok_tr.to_rust_type();
                    writeln!(out, "{indent}    Ok(unsafe {{ <{ty} as FfiType>::from_c(__result.assume_init()) }})").unwrap();
                    writeln!(out, "{indent}}} else {{").unwrap();
                    writeln!(
                        out,
                        "{indent}    Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                    )
                    .unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                }
                CResultConvention::OptionalValueOutParam => {
                    let ty_static = optional_value_out_type(ok_tr);
                    let mut bare = ok_tr.clone();
                    bare.optional = false;
                    let ty = bare.to_rust_type();
                    writeln!(out, "{indent}let mut __result_is_some = false;").unwrap();
                    writeln!(out, "{indent}let mut __result = core::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();").unwrap();
                    writeln!(out, "{indent}let __r = unsafe {{ {ffi_name}({args_str}{sep}&mut __result_is_some as *mut bool, __result.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
                    writeln!(out, "{indent}if __r == 0 {{").unwrap();
                    writeln!(out, "{indent}    if __result_is_some {{").unwrap();
                    writeln!(out, "{indent}        Ok(Some(unsafe {{ <{ty} as FfiType>::from_c(__result.assume_init()) }}))").unwrap();
                    writeln!(out, "{indent}    }} else {{").unwrap();
                    writeln!(out, "{indent}        Ok(None)").unwrap();
                    writeln!(out, "{indent}    }}").unwrap();
                    writeln!(out, "{indent}}} else {{").unwrap();
                    writeln!(
                        out,
                        "{indent}    Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                    )
                    .unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                }
            }
        }
    }
}
