//! Format safe wrapper and trait method signatures.

use std::fmt::Write;

use ffier_schema::{Library, Method, ParamType, Receiver, Return};

/// Check if a Return is a borrowed handle: `&HandleType` or `Result<&HandleType, E>`.
/// Returns the bare type name (e.g. "Gadget") if so.
pub(super) fn borrowed_handle_return_type<'a>(ret: &'a Return, lib: &Library) -> Option<&'a str> {
    let tr = match ret {
        Return::Value(tr) => tr,
        Return::Result { ok: Some(tr), .. } => tr,
        _ => return None,
    };
    if tr.ref_kind != ffier_schema::RefKind::Shared && tr.ref_kind != ffier_schema::RefKind::Mut {
        return None;
    }
    lib.type_entry(&tr.type_name)
        .filter(|e| e.kind.is_handle())
        .map(|_| tr.type_name.as_str())
}

pub(super) fn format_receiver(m: &Method) -> String {
    let lifetime = m
        .receiver_lifetime
        .as_ref()
        .map(|lt| format!("'{lt} "))
        .unwrap_or_default();
    match m.receiver {
        Receiver::None => String::new(),
        Receiver::Ref => format!("&{lifetime}self, "),
        Receiver::Mut => format!("&{lifetime}mut self, "),
        Receiver::Value => "self, ".to_string(),
    }
}

pub(super) fn build_wrapper_return_type(m: &Method, lib: &Library) -> String {
    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    if is_builder {
        return match m.receiver {
            Receiver::Mut => " -> &mut Self".to_string(),
            Receiver::Value => format!(" -> {}", m.ret.to_rust_type(&lib.type_registry)),
            _ => format!(" -> {}", m.ret.to_rust_type(&lib.type_registry)),
        };
    }

    // Borrowed handle returns (&HandleType) → return owned wrapper in client.
    // The wrapper's Drop calls destroy, which deallocates the borrowed shell
    // without dropping the inner value.
    if let Some(bare_name) = borrowed_handle_return_type(&m.ret, lib) {
        return match &m.ret {
            Return::Value(_) => format!(" -> {bare_name}"),
            Return::Result { err_type, .. } => format!(" -> Result<{bare_name}, {err_type}>"),
            _ => unreachable!(),
        };
    }

    match &m.ret {
        Return::Void => String::new(),
        Return::ObjectArray { element } => {
            format!(" -> ForeignSlice<{}>", element.type_name)
        }
        Return::Value(_) | Return::OptionalValue(_) | Return::Result { .. } => {
            format!(" -> {}", m.ret.to_rust_type(&lib.type_registry))
        }
    }
}

pub(super) fn build_trait_method_sig(m: &Method, lib: &Library) -> String {
    let receiver_str = format_receiver(m);

    let params: Vec<String> = m.params.iter().map(format_param_sig).collect();
    let params_str = params.join(", ");

    let ret = match &m.ret {
        Return::Void => String::new(),
        _ => format!(" -> {}", m.ret.to_rust_type(&lib.type_registry)),
    };

    let lifetimes = lifetime_generics(&m.method_lifetimes);
    let attr = if m.receiver_lifetime.is_some() {
        "#[allow(clippy::needless_lifetimes)]\n    "
    } else {
        ""
    };
    format!(
        "{attr}fn {}{lifetimes}({receiver_str}{params_str}){ret}",
        m.name
    )
}

pub(super) fn lifetime_generics(lifetimes: &[String]) -> String {
    if lifetimes.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            lifetimes
                .iter()
                .map(|lt| format!("'{lt}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

pub(super) fn format_param_sig(p: &ffier_schema::Param) -> String {
    match &p.param_type {
        ParamType::Regular(tr) => format!("{}: {}", p.name, tr.to_rust_type()),
        ParamType::Slice { element, .. } => {
            format!("{}: &[{}]", p.name, element.to_rust_type())
        }
        ParamType::ImplTrait {
            trait_name,
            type_args,
            lifetime_bounds,
            ref_kind,
            ref_lifetime,
        } => {
            let mut ty = if type_args.is_empty() {
                format!("impl {trait_name}")
            } else {
                let args = type_args
                    .iter()
                    .map(|lt| format!("'{lt}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("impl {trait_name}<{args}>")
            };
            for lt in lifetime_bounds {
                write!(ty, " + '{lt}").unwrap();
            }
            if *ref_kind != ffier_schema::RefKind::None {
                if !lifetime_bounds.is_empty() {
                    ty = format!("({ty})");
                }
                let lifetime = ref_lifetime
                    .as_ref()
                    .map(|lt| format!("'{lt} "))
                    .unwrap_or_default();
                let mutability = if *ref_kind == ffier_schema::RefKind::Mut {
                    "mut "
                } else {
                    ""
                };
                ty = format!("&{lifetime}{mutability}{ty}");
            }
            format!("{}: {ty}", p.name)
        }
    }
}
