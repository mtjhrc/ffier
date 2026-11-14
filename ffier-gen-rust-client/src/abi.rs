//! Shared C ABI parameters, return types, and out-parameters.

use ffier_schema::{ParamType, Return};

/// Append return-type out-params and return the return-type string for an
/// extern "C" function. Handles Void, Value, Result (FfierResult-style with
/// out-params and GLib-style for handle-returning Results).
///
/// Shared by all extern signature builders.
pub(crate) fn push_return_and_out_params(
    ret: &Return,
    params: &mut Vec<String>,
    is_builder: bool,
) -> String {
    use ffier_schema::CResultConvention;
    match ret {
        Return::Void => String::new(),
        Return::Value(_) if is_builder => String::new(),
        Return::ObjectArray { .. } => " -> ffier::FfierObjectArray".to_string(),
        Return::Value(tr) => {
            let ty = tr.to_rust_type_static();
            format!(" -> <{ty} as FfiType>::CRepr")
        }
        Return::OptionalValue(tr) => {
            let ty = optional_value_out_type(tr);
            params.push(format!("result: *mut <{ty} as FfiType>::CRepr"));
            " -> bool".to_string()
        }
        Return::Result {
            ok, c_convention, ..
        } => match c_convention {
            CResultConvention::HandleOrNull => {
                params.push("err_out: *mut *mut core::ffi::c_void".to_string());
                " -> *mut core::ffi::c_void".to_string()
            }
            CResultConvention::OutParam => {
                if let Some(ok_tr) = ok.as_ref().filter(|_| !is_builder) {
                    let ty = ok_tr.to_rust_type_static();
                    params.push(format!("result: *mut <{ty} as FfiType>::CRepr"));
                }
                params.push("err_out: *mut *mut core::ffi::c_void".to_string());
                " -> ffier::FfierResult".to_string()
            }
            CResultConvention::OptionalValueOutParam => {
                let ok_tr = ok
                    .as_ref()
                    .expect("OptionalValueOutParam requires an ok type");
                let ty = optional_value_out_type(ok_tr);
                params.push("result_is_some: *mut bool".to_string());
                params.push(format!("result: *mut <{ty} as FfiType>::CRepr"));
                params.push("err_out: *mut *mut core::ffi::c_void".to_string());
                " -> ffier::FfierResult".to_string()
            }
        },
    }
}

/// Rust type rendering for an optional-value out-param.
///
/// The schema marks the ok type `optional` so generated wrapper signatures can
/// say `Option<T>`, but the C out-param is a bare `T`.
pub(crate) fn optional_value_out_type(tr: &ffier_schema::TypeRef) -> String {
    let mut bare = tr.clone();
    bare.optional = false;
    bare.to_rust_type_static()
}

/// Append C extern param strings (`"name: <Type as FfiType>::CRepr"` etc.)
/// for a list of schema params. Shared by all extern signature builders.
pub(crate) fn push_extern_params(params: &[ffier_schema::Param], out: &mut Vec<String>) {
    for p in params {
        match &p.param_type {
            ParamType::Regular(tr) => {
                let ty = tr.to_rust_type_static();
                out.push(format!("{}: <{ty} as FfiType>::CRepr", p.name));
            }
            ParamType::Slice { element, .. } => {
                let elem_ty = element.to_rust_type_static();
                out.push(format!("{}: *const <{elem_ty} as FfiType>::CRepr", p.name));
                out.push(format!("{}_len: usize", p.name));
            }
            ParamType::ImplTrait { .. } => {
                out.push(format!("{}: *mut core::ffi::c_void", p.name));
            }
        }
    }
}

/// Append return type-only strings (no param names) for vtable fn pointer
/// fields. Companion to `push_extern_param_types`.
pub(crate) fn push_return_and_out_param_types(ret: &Return, out: &mut Vec<String>) -> String {
    use ffier_schema::CResultConvention;
    match ret {
        Return::Void => String::new(),
        Return::ObjectArray { .. } => " -> ffier::FfierObjectArray".to_string(),
        Return::Value(tr) => {
            let ty = tr.to_rust_type_static();
            format!(" -> <{ty} as FfiType>::CRepr")
        }
        Return::OptionalValue(tr) => {
            let ty = optional_value_out_type(tr);
            out.push(format!("*mut <{ty} as FfiType>::CRepr"));
            " -> bool".to_string()
        }
        Return::Result {
            ok, c_convention, ..
        } => match c_convention {
            CResultConvention::HandleOrNull => {
                out.push("*mut *mut core::ffi::c_void".to_string());
                " -> *mut core::ffi::c_void".to_string()
            }
            CResultConvention::OutParam => {
                if let Some(ok_tr) = ok {
                    let ty = ok_tr.to_rust_type_static();
                    out.push(format!("*mut <{ty} as FfiType>::CRepr"));
                }
                out.push("*mut *mut core::ffi::c_void".to_string());
                " -> ffier::FfierResult".to_string()
            }
            CResultConvention::OptionalValueOutParam => {
                let ok_tr = ok
                    .as_ref()
                    .expect("OptionalValueOutParam requires an ok type");
                let ty = optional_value_out_type(ok_tr);
                out.push("*mut bool".to_string());
                out.push(format!("*mut <{ty} as FfiType>::CRepr"));
                out.push("*mut *mut core::ffi::c_void".to_string());
                " -> ffier::FfierResult".to_string()
            }
        },
    }
}

/// Append C type-only strings (no param names) for vtable struct fields.
pub(crate) fn push_extern_param_types(params: &[ffier_schema::Param], out: &mut Vec<String>) {
    for p in params {
        match &p.param_type {
            ParamType::Regular(tr) => {
                let ty = tr.to_rust_type_static();
                out.push(format!("<{ty} as FfiType>::CRepr"));
            }
            ParamType::Slice { element, .. } => {
                let elem_ty = element.to_rust_type_static();
                out.push(format!("*const <{elem_ty} as FfiType>::CRepr"));
                out.push("usize".to_string());
            }
            ParamType::ImplTrait { .. } => {
                out.push("*mut core::ffi::c_void".to_string());
            }
        }
    }
}
