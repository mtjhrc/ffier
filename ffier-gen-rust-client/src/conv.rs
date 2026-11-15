//! How each client value crosses the C ABI, defined once for both
//! directions.
//!
//! A library call (`calls.rs`) lowers args and lifts the return; a
//! callback trampoline (`traits/vtable.rs`) lifts params and lowers the
//! return. Both go through the functions here, one per direction and scope,
//! so every type kind is converted the same way in either role.

use ffier_schema::{Library, Param, ParamType, RefKind, TypeRef};

use crate::ffi_traits::{self, TypeRefExt};

/// Statements to emit before the call, and the converted expressions.
#[derive(Default)]
pub(crate) struct Converted {
    pub(crate) pre: Vec<String>,
    /// One expression per C value (a slice lowers to pointer and length).
    pub(crate) exprs: Vec<String>,
}

impl Converted {
    fn expr(expr: String) -> Self {
        Self {
            pre: Vec::new(),
            exprs: vec![expr],
        }
    }
}

/// A `&Handle` / `&mut Handle`: the client wrapper lends the pointer it
/// holds. Returns the token of the handle's impls.
fn borrowed_handle(tr: &TypeRef, lib: &Library) -> Option<String> {
    let is_handle = lib
        .type_entry(&tr.type_name)
        .is_some_and(|e| e.kind.is_handle());
    (tr.ref_kind != RefKind::None && is_handle).then(|| ffi_traits::token(tr, lib))
}

/// Lower an argument passed to a library call, for the duration of the call.
pub(crate) fn lower_arg(p: &Param, lib: &Library) -> Converted {
    let name = &p.name;
    match &p.param_type {
        // `as_handle` keeps the borrow's lifetime; `into_c` would let the
        // pointer escape it.
        ParamType::Regular(tr) => Converted::expr(match borrowed_handle(tr, lib) {
            Some(tok) => format!("ffier::ClientHandle::<{tok}>::as_handle({name})"),
            None => tr.convert_into_c(lib, name),
        }),
        ParamType::Slice { element, .. } => {
            if element.ref_kind == RefKind::None {
                // Primitives are layout-compatible with their CRepr.
                return Converted {
                    pre: Vec::new(),
                    exprs: vec![format!("{name}.as_ptr()"), format!("{name}.len()")],
                };
            }
            // Ref elements (strings): convert each into a C array.
            let tok = ffi_traits::token(element, lib);
            let elem_ty = element.to_rust_type();
            let binding = format!("__ffi_{name}");
            Converted {
                pre: vec![format!(
                    "let {binding}: Vec<{}> = {name}.iter().map(|s| {}).collect();",
                    ffi_traits::c_repr(&elem_ty, &tok),
                    ffi_traits::into_c(&elem_ty, &tok, "*s"),
                )],
                exprs: vec![format!("{binding}.as_ptr()"), format!("{binding}.len()")],
            }
        }
        // An owned callback transfers its handle; a borrowed one keeps a
        // guard alive for the call.
        ParamType::ImplTrait { ref_kind, .. } => {
            let method = match ref_kind {
                RefKind::None => {
                    return Converted::expr(format!("{name}.__into_callback_handle().into_raw()"));
                }
                RefKind::Shared => "__borrow_callback_handle",
                RefKind::Mut => "__borrow_callback_handle_mut",
            };
            let binding = format!("__ffi_{name}");
            Converted {
                pre: vec![format!("let {binding} = {name}.{method}();")],
                exprs: vec![format!("{binding}.as_raw()")],
            }
        }
    }
}

/// Lift a parameter a trampoline received from C. Params are named after
/// the schema's C params (see `abi::push_extern_params`).
pub(crate) fn lift_param(p: &Param, lib: &Library) -> Converted {
    let name = &p.name;
    match &p.param_type {
        ParamType::Regular(tr) => Converted::expr(match borrowed_handle(tr, lib) {
            Some(_) => format!(
                "compile_error!(\"ffier: Rust callbacks cannot receive a borrowed handle `{}` yet\")",
                tr.type_name
            ),
            None => format!("unsafe {{ {} }}", tr.convert_from_c(lib, name)),
        }),
        // The raw callback handle is passed through.
        ParamType::ImplTrait { .. } => Converted::expr(name.clone()),
        ParamType::Slice { element, c_params } => {
            let [ptr, len] = c_params.as_slice() else {
                panic!("slice param `{name}` must expand to pointer + length");
            };
            let (ptr, len) = (&ptr.name, &len.name);
            let binding = format!("__slice_{name}");
            let elem_ty = element.to_rust_type();
            if element.ref_kind == RefKind::None {
                // Primitives: the C array is the slice.
                Converted {
                    pre: vec![format!(
                        "let {binding} = if {len} == 0 {{ &[] as &[{elem_ty}] }} else {{ unsafe {{ core::slice::from_raw_parts({ptr}, {len}) }} }};"
                    )],
                    exprs: vec![binding],
                }
            } else {
                // Ref elements: convert each C value back.
                Converted {
                    pre: vec![format!(
                        "let {binding}: Vec<{elem_ty}> = unsafe {{ core::slice::from_raw_parts({ptr}, {len}) }}.iter().map(|v| unsafe {{ {} }}).collect();",
                        element.convert_from_c(lib, "*v")
                    )],
                    exprs: vec![format!("&{binding}")],
                }
            }
        }
    }
}

/// Lower a value a trampoline returns to C.
pub(crate) fn lower_returned(tr: &TypeRef, lib: &Library, value: &str) -> String {
    match borrowed_handle(tr, lib) {
        Some(tok) => format!("ffier::ClientHandle::<{tok}>::as_handle({value})"),
        None => tr.convert_into_c(lib, value),
    }
}

/// Lift a value a library call returned. A `&Handle` becomes the owned
/// wrapper when `own_borrowed_handles` (inherent methods, whose signature
/// returns the wrapper); trait methods cannot produce one yet.
pub(crate) fn lift_returned(
    tr: &TypeRef,
    lib: &Library,
    raw: &str,
    own_borrowed_handles: bool,
) -> String {
    match borrowed_handle(tr, lib) {
        None => format!("unsafe {{ {} }}", tr.convert_from_c(lib, raw)),
        Some(tok) if own_borrowed_handles => format!(
            "unsafe {{ <{} as ffier::ClientHandle<{tok}>>::from_raw({raw}) }}",
            tr.type_name
        ),
        // A client wrapper is not stored where a `&Handle` could point;
        // producing one needs a borrow type that keeps the lifetime.
        Some(_) => format!(
            "{{ let _ = {raw}; unimplemented!(\"ffier: Rust clients cannot yet return `&{}` from a trait method\") }}",
            tr.type_name
        ),
    }
}
