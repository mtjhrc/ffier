//! Format safe wrapper and trait method signatures.

use std::fmt::Write;

use ffier_schema::{Library, Method, ParamType, Receiver, Return};

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

/// ` -> T` for a wrapper returning the schema's Rust type, or empty for void.
pub(super) fn plain_return_type(ret: &Return, lib: &Library) -> String {
    match ret {
        Return::Void => String::new(),
        _ => format!(" -> {}", ret.to_rust_type(&lib.type_registry)),
    }
}

/// Emit `#[doc = "..."]` attributes, escaping each line for a string literal.
pub(super) fn emit_doc_attrs(out: &mut String, doc: &[String], indent: &str) {
    for line in doc {
        let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(out, "{indent}#[doc = \"{escaped}\"]").unwrap();
    }
}

pub(super) fn build_trait_method_sig(m: &Method, lib: &Library) -> String {
    let receiver_str = format_receiver(m);

    let params: Vec<String> = m.params.iter().map(format_param_sig).collect();
    let params_str = params.join(", ");

    let ret = plain_return_type(&m.ret, lib);

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
