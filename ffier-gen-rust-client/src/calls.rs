//! Marshal safe wrapper arguments and results around FFI calls.

use std::fmt::Write;

use ffier_schema::{CResultConvention, Library, ParamType, Return, TypeRef};

use crate::abi::optional_value_out_type;
use crate::error::find_error_dispatch_fns;
use crate::signature::borrowed_handle_return_type;

/// Build FFI call arguments from method params. Each param is converted
/// via `FfiType::into_c`, `as_handle()`, `__into_raw_handle()`, or pre-bound
/// slice reference. For borrowed handle params (`&Handle`), uses `as_handle()`
/// instead of `into_c()` to avoid lifetime escape.
fn build_ffi_param_args(params: &[ffier_schema::Param], lib: &Library) -> Vec<String> {
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
fn emit_param_pre_bindings(out: &mut String, params: &[ffier_schema::Param], indent: &str) {
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

/// How a safe wrapper turns the FFI result into its Rust return value.
#[derive(Clone, Copy, Default)]
pub(crate) struct ReturnPolicy {
    /// Set for builder methods returning `Self` (`-> Self` /
    /// `-> Result<Self, E>`). The bridge writes the new handle back instead
    /// of returning a value, so the wrapper rebuilds `Self` itself.
    pub(crate) builder: Option<BuilderReceiver>,
    /// Surface `&Handle` returns as the owned wrapper via
    /// `FfiHandle::__from_raw`. Only inherent methods set this, because
    /// `build_wrapper_return_type` rewrites their signature to the owned
    /// wrapper; trait signatures keep `&T`.
    pub(crate) own_borrowed_handles: bool,
}

/// Receiver of a builder method, which decides how `Self` is produced.
#[derive(Clone, Copy)]
pub(crate) enum BuilderReceiver {
    /// `&mut self` — the handle is updated in place; return `self`.
    Mut,
    /// `self` — the bridge wrote the new handle into `__handle`.
    Value { has_lifetimes: bool },
}

impl BuilderReceiver {
    fn self_expr(self) -> &'static str {
        match self {
            BuilderReceiver::Mut => "self",
            BuilderReceiver::Value {
                has_lifetimes: true,
            } => "Self(__handle, std::marker::PhantomData)",
            BuilderReceiver::Value {
                has_lifetimes: false,
            } => "Self(__handle)",
        }
    }
}

const ERR_DECL: &str = "let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();";
const ERR_ARG: &str = "&mut __err as *mut *mut core::ffi::c_void";

/// Emit a safe wrapper body that forwards to `ffi_name`: param pre-bindings,
/// the call with `self_arg` (if any) followed by the converted params, and the
/// conversion of the result into the wrapper's return value.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_call_body(
    out: &mut String,
    ffi_name: &str,
    self_arg: Option<&str>,
    params: &[ffier_schema::Param],
    ret: &Return,
    indent: &str,
    lib: &Library,
    policy: ReturnPolicy,
) {
    emit_param_pre_bindings(out, params, indent);
    let args: Vec<String> = self_arg
        .map(str::to_string)
        .into_iter()
        .chain(build_ffi_param_args(params, lib))
        .collect();
    emit_ffi_call_return(out, ffi_name, &args.join(", "), ret, indent, lib, policy);
}

/// Emit an FFI call and convert its result into the wrapper's return value.
///
/// This is the only place that knows how each [`Return`] /
/// [`CResultConvention`] pair is decoded on the client side.
fn emit_ffi_call_return(
    out: &mut String,
    ffi_name: &str,
    args_str: &str,
    ret: &Return,
    indent: &str,
    lib: &Library,
    policy: ReturnPolicy,
) {
    let i = indent;
    // The call expression, with return out-params appended to the args.
    let call = |out_params: &[&str]| -> String {
        let args = std::iter::once(args_str)
            .filter(|a| !a.is_empty())
            .chain(out_params.iter().copied())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{ffi_name}({args})")
    };
    let err = |err_type: &str| format!("Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})");
    // Convert a raw C value into the Rust ok type.
    let from_c = |tr: &TypeRef, raw: &str| -> String {
        if policy.own_borrowed_handles && borrowed_handle_return_type(ret, lib).is_some() {
            // Borrowed handle: wrap the shell in the owned wrapper.
            format!(
                "unsafe {{ <{} as FfiHandle>::__from_raw({raw}) }}",
                tr.type_name
            )
        } else {
            format!(
                "unsafe {{ <{} as FfiType>::from_c({raw}) }}",
                tr.to_rust_type()
            )
        }
    };
    let builder = policy
        .builder
        .filter(|_| ret.is_builder_self(&lib.type_registry));

    match (ret, builder) {
        (Return::Value(_), Some(b)) => {
            writeln!(out, "{i}unsafe {{ {} }};", call(&[])).unwrap();
            writeln!(out, "{i}{}", b.self_expr()).unwrap();
        }
        (Return::Result { err_type, .. }, Some(b)) => {
            writeln!(out, "{i}{ERR_DECL}").unwrap();
            writeln!(out, "{i}let __r = unsafe {{ {} }};", call(&[ERR_ARG])).unwrap();
            writeln!(
                out,
                "{i}if __r == 0 {{ Ok({}) }} else {{ {} }}",
                b.self_expr(),
                err(err_type)
            )
            .unwrap();
        }
        (Return::Void, _) => {
            writeln!(out, "{i}unsafe {{ {} }}", call(&[])).unwrap();
        }
        (Return::Value(tr), None) => {
            writeln!(out, "{i}let __raw = unsafe {{ {} }};", call(&[])).unwrap();
            writeln!(out, "{i}{}", from_c(tr, "__raw")).unwrap();
        }
        (Return::OptionalValue(tr), _) => {
            let ty_static = optional_value_out_type(tr);
            writeln!(
                out,
                "{i}let mut __out = std::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();"
            )
            .unwrap();
            writeln!(
                out,
                "{i}let __is_some = unsafe {{ {} }};",
                call(&["__out.as_mut_ptr()"])
            )
            .unwrap();
            writeln!(out, "{i}if __is_some {{").unwrap();
            let value = from_c(&tr.without_optional(), "__out.assume_init()");
            writeln!(out, "{i}    Some({value})").unwrap();
            writeln!(out, "{i}}} else {{").unwrap();
            writeln!(out, "{i}    None").unwrap();
            writeln!(out, "{i}}}").unwrap();
        }
        (
            Return::Result {
                ok: None, err_type, ..
            },
            None,
        ) => {
            writeln!(out, "{i}{ERR_DECL}").unwrap();
            writeln!(out, "{i}let __r = unsafe {{ {} }};", call(&[ERR_ARG])).unwrap();
            writeln!(
                out,
                "{i}if __r == 0 {{ Ok(()) }} else {{ {} }}",
                err(err_type)
            )
            .unwrap();
        }
        (
            Return::Result {
                ok: Some(ok_tr),
                err_type,
                c_convention,
            },
            None,
        ) => {
            writeln!(out, "{i}{ERR_DECL}").unwrap();
            match c_convention {
                CResultConvention::HandleOrNull => {
                    // The handle is the return value, NULL on error; the
                    // error code is recovered from the error handle.
                    let (error_result_fn, _) = find_error_dispatch_fns(lib);
                    writeln!(out, "{i}let __raw = unsafe {{ {} }};", call(&[ERR_ARG])).unwrap();
                    writeln!(out, "{i}if !__raw.is_null() {{").unwrap();
                    writeln!(out, "{i}    Ok({})", from_c(ok_tr, "__raw")).unwrap();
                    writeln!(out, "{i}}} else {{").unwrap();
                    writeln!(
                        out,
                        "{i}    let __r = unsafe {{ {error_result_fn}(__err) }};"
                    )
                    .unwrap();
                    writeln!(out, "{i}    {}", err(err_type)).unwrap();
                    writeln!(out, "{i}}}").unwrap();
                }
                CResultConvention::OutParam => {
                    let ty_static = ok_tr.to_rust_type_static();
                    writeln!(out, "{i}let mut __result = core::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();").unwrap();
                    writeln!(
                        out,
                        "{i}let __r = unsafe {{ {} }};",
                        call(&["__result.as_mut_ptr()", ERR_ARG])
                    )
                    .unwrap();
                    writeln!(out, "{i}if __r == 0 {{").unwrap();
                    let value = from_c(ok_tr, "__result.assume_init()");
                    writeln!(out, "{i}    Ok({value})").unwrap();
                    writeln!(out, "{i}}} else {{").unwrap();
                    writeln!(out, "{i}    {}", err(err_type)).unwrap();
                    writeln!(out, "{i}}}").unwrap();
                }
                CResultConvention::OptionalValueOutParam => {
                    // Presence flag plus a separate value out-param: `Ok(None)`
                    // leaves the value buffer untouched, and errors write only
                    // `err_out`.
                    let ty_static = optional_value_out_type(ok_tr);
                    writeln!(out, "{i}let mut __result_is_some = false;").unwrap();
                    writeln!(out, "{i}let mut __result = core::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();").unwrap();
                    writeln!(
                        out,
                        "{i}let __r = unsafe {{ {} }};",
                        call(&[
                            "&mut __result_is_some as *mut bool",
                            "__result.as_mut_ptr()",
                            ERR_ARG
                        ])
                    )
                    .unwrap();
                    writeln!(out, "{i}if __r == 0 {{").unwrap();
                    writeln!(out, "{i}    if __result_is_some {{").unwrap();
                    let value = from_c(&ok_tr.without_optional(), "__result.assume_init()");
                    writeln!(out, "{i}        Ok(Some({value}))").unwrap();
                    writeln!(out, "{i}    }} else {{").unwrap();
                    writeln!(out, "{i}        Ok(None)").unwrap();
                    writeln!(out, "{i}    }}").unwrap();
                    writeln!(out, "{i}}} else {{").unwrap();
                    writeln!(out, "{i}    {}", err(err_type)).unwrap();
                    writeln!(out, "{i}}}").unwrap();
                }
            }
        }
    }
}
