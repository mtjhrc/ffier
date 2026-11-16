//! Marshal safe wrapper arguments and results around FFI calls.

use std::fmt::Write;

use ffier_schema::{CResultConvention, Library, Return, TypeRef};

use crate::abi::optional_value_out_type;
use crate::error::find_error_dispatch_fns;
use crate::ffi_traits::{self, TypeRefExt};

const ERR_DECL: &str = "let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();";
const ERR_ARG: &str = "&mut __err as *mut *mut core::ffi::c_void";

/// Emit a safe wrapper body that forwards to `ffi_name`: param pre-bindings,
/// the call with `self_arg` (if any) followed by the converted params, and the
/// conversion of the result into the wrapper's return value.
/// `receiver_return` gives the Rust success expression for an annotated
/// receiver return.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_call_body(
    out: &mut String,
    ffi_name: &str,
    self_arg: Option<&str>,
    params: &[ffier_schema::Param],
    ret: &Return,
    indent: &str,
    lib: &Library,
    receiver_return: Option<&str>,
) {
    let mut args: Vec<String> = self_arg.map(str::to_string).into_iter().collect();
    for p in params {
        let converted = crate::conv::lower_arg(p, lib);
        for pre in converted.pre {
            writeln!(out, "{indent}{pre}").unwrap();
        }
        args.extend(converted.exprs);
    }
    emit_ffi_call_return(
        out,
        ffi_name,
        &args.join(", "),
        ret,
        indent,
        lib,
        receiver_return,
    );
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
    receiver_return: Option<&str>,
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
    let from_c = |tr: &TypeRef, raw: &str| format!("unsafe {{ {} }}", tr.convert_from_c(lib, raw));
    match (ret, receiver_return) {
        (Return::Value(_), Some(receiver_return)) => {
            writeln!(out, "{i}unsafe {{ {} }};", call(&[])).unwrap();
            writeln!(out, "{i}{receiver_return}").unwrap();
        }
        (Return::Result { err_type, .. }, Some(receiver_return)) => {
            writeln!(out, "{i}{ERR_DECL}").unwrap();
            writeln!(out, "{i}let __r = unsafe {{ {} }};", call(&[ERR_ARG])).unwrap();
            writeln!(
                out,
                "{i}if __r == 0 {{ Ok({receiver_return}) }} else {{ {} }}",
                err(err_type)
            )
            .unwrap();
        }
        (Return::Void, _) => {
            writeln!(out, "{i}unsafe {{ {} }}", call(&[])).unwrap();
        }
        // The callee must not return; never manufacture an uninhabited value.
        (Return::Never, _) => {
            writeln!(out, "{i}unsafe {{ {} }};", call(&[])).unwrap();
            writeln!(out, "{i}std::process::abort()").unwrap();
        }
        (Return::Value(tr), None) => {
            writeln!(out, "{i}let __raw = unsafe {{ {} }};", call(&[])).unwrap();
            writeln!(out, "{i}{}", from_c(tr, "__raw")).unwrap();
        }
        (Return::OptionalValue(tr), _) => {
            let ty_static = optional_value_out_type(tr);
            writeln!(
                out,
                "{i}let mut __out = std::mem::MaybeUninit::<{}>::uninit();",
                ffi_traits::c_repr(&ty_static, &ffi_traits::token(tr, lib))
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
        )
        | (Return::ResultNeverOk { err_type }, _) => {
            let success = if matches!(ret, Return::ResultNeverOk { .. }) {
                "std::process::abort()"
            } else {
                "Ok(())"
            };
            writeln!(out, "{i}{ERR_DECL}").unwrap();
            writeln!(out, "{i}let __r = unsafe {{ {} }};", call(&[ERR_ARG])).unwrap();
            writeln!(
                out,
                "{i}if __r == 0 {{ {success} }} else {{ {} }}",
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
                    writeln!(
                        out,
                        "{i}let mut __result = core::mem::MaybeUninit::<{}>::uninit();",
                        ok_tr.c_repr_static(lib)
                    )
                    .unwrap();
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
                    writeln!(
                        out,
                        "{i}let mut __result = core::mem::MaybeUninit::<{}>::uninit();",
                        ffi_traits::c_repr(&ty_static, &ffi_traits::token(ok_tr, lib))
                    )
                    .unwrap();
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
