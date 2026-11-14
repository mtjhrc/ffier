//! Shared C ABI parameters, return types, and out-parameters.

use ffier_schema::{AbiOutParam, AbiReturn, Library, Param, ParamType, Return, TypeRef};

use crate::ffi_traits::TypeRefExt;

/// Whether ABI parameter lists carry names (`name: Type`, for `extern "C"`
/// declarations and trampolines) or only types (for fn-pointer fields).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParamNames {
    Named,
    TypesOnly,
}

impl ParamNames {
    fn render(self, name: &str, ty: String) -> String {
        match self {
            ParamNames::Named => format!("{name}: {ty}"),
            ParamNames::TypesOnly => ty,
        }
    }
}

/// Append the return out-params to `out` and return the extern return type
/// (` -> T`, or empty for void), following [`Return::c_abi`].
pub(crate) fn push_return_and_out_params(
    ret: &Return,
    out: &mut Vec<String>,
    lib: &Library,
    names: ParamNames,
) -> String {
    let (abi_ret, out_params) = ret.c_abi(&lib.type_registry);
    for p in &out_params {
        let ty = match p {
            AbiOutParam::Result(tr) => format!("*mut {}", tr.c_repr_static(lib)),
            AbiOutParam::ResultIsSome => "*mut bool".to_string(),
            AbiOutParam::ErrOut => "*mut *mut core::ffi::c_void".to_string(),
        };
        out.push(names.render(p.name(), ty));
    }
    match abi_ret {
        AbiReturn::Void => String::new(),
        AbiReturn::Value(tr) => format!(" -> {}", tr.c_repr_static(lib)),
        AbiReturn::Bool => " -> bool".to_string(),
        AbiReturn::Handle(_) => " -> *mut core::ffi::c_void".to_string(),
        AbiReturn::FfierResult => " -> ffier::FfierResult".to_string(),
    }
}

/// Rust type rendering for an optional-value out-param.
///
/// The schema marks the ok type `optional` so generated wrapper signatures can
/// say `Option<T>`, but the C out-param is a bare `T`.
pub(crate) fn optional_value_out_type(tr: &TypeRef) -> String {
    tr.without_optional().to_rust_type_static()
}

/// Append the extern params for a list of schema params. A slice expands to
/// the pointer and length params the schema records in `c_params`.
pub(crate) fn push_extern_params(
    params: &[Param],
    out: &mut Vec<String>,
    lib: &Library,
    names: ParamNames,
) {
    for p in params {
        match &p.param_type {
            ParamType::Regular(tr) => out.push(names.render(&p.name, tr.c_repr_static(lib))),
            ParamType::Slice { element, c_params } => {
                let [ptr, len] = c_params.as_slice() else {
                    panic!("slice param `{}` must expand to pointer + length", p.name);
                };
                out.push(names.render(&ptr.name, format!("*const {}", element.c_repr_static(lib))));
                out.push(names.render(&len.name, "usize".to_string()));
            }
            ParamType::ImplTrait { .. } => {
                out.push(names.render(&p.name, "*mut core::ffi::c_void".to_string()));
            }
        }
    }
}
