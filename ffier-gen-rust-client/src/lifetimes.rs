//! Preserve reference elision when handle inputs become `impl Into<Ref<...>>`.

use std::collections::HashSet;

use ffier_schema::{
    FreeFunction, Library, Method, Param, ParamType, Receiver, RefKind, Return, TypeRef,
};

pub(super) fn method(method: &Method, library: &Library) -> Method {
    let mut method = method.clone();
    normalize(
        &mut method.params,
        &mut method.ret,
        &mut method.method_lifetimes,
        method.receiver,
        &mut method.receiver_lifetime,
        library,
    );
    method
}

pub(super) fn free_function(function: &FreeFunction, library: &Library) -> FreeFunction {
    let mut function = function.clone();
    normalize(
        &mut function.params,
        &mut function.ret,
        &mut function.lifetimes,
        Receiver::None,
        &mut None,
        library,
    );
    function
}

/// Restore the well-formedness bounds implied by native `&'a T` inputs.
pub(super) fn input_bounds(params: &[Param], library: &Library) -> String {
    let mut bounds = Vec::new();
    let mut seen = HashSet::new();
    for param in params {
        let ParamType::Regular(ty) = &param.param_type else {
            continue;
        };
        if ty.ref_kind == RefKind::None
            || !library
                .type_entry(&ty.type_name)
                .is_some_and(|entry| entry.kind.is_handle())
            || ty.type_args.is_empty()
        {
            continue;
        }
        let Some(lifetime) = ty
            .ref_lifetime
            .as_deref()
            .filter(|lifetime| *lifetime != "_")
        else {
            // Callback trampolines infer call-local anonymous borrows. Public
            // Into inputs have already received names during normalization.
            continue;
        };
        let mut bare = ty.clone();
        bare.ref_kind = RefKind::None;
        bare.ref_lifetime = None;
        bare.optional = false;
        bare.owned = false;
        let bound = format!("{}: '{lifetime}", bare.to_rust_type());
        if seen.insert(bound.clone()) {
            bounds.push(bound);
        }
    }
    if bounds.is_empty() {
        String::new()
    } else {
        format!(" where {}", bounds.join(", "))
    }
}

fn normalize(
    params: &mut [Param],
    ret: &mut Return,
    generics: &mut Vec<String>,
    receiver: Receiver,
    receiver_lifetime: &mut Option<String>,
    library: &Library,
) {
    let borrowed_handle = |ty: &TypeRef| {
        ty.ref_kind != RefKind::None
            && library
                .type_entry(&ty.type_name)
                .is_some_and(|entry| entry.kind.is_handle())
    };
    // Concrete Ref inputs and outputs still support Rust's lifetime elision.
    // Only the Into bounds introduced for handle inputs hide input lifetimes.
    if !params
        .iter()
        .any(|param| matches!(&param.param_type, ParamType::Regular(ty) if borrowed_handle(ty)))
    {
        return;
    }

    let mut used: HashSet<String> = generics.iter().cloned().collect();
    used.extend(receiver_lifetime.iter().cloned());
    for entry in library.type_registry.values() {
        used.extend(entry.lifetime_params.iter().map(|param| param.name.clone()));
    }
    used.extend(
        library
            .trait_impls
            .iter()
            .flat_map(|implementation| implementation.lifetimes.iter().cloned()),
    );
    for param in params.iter() {
        match &param.param_type {
            ParamType::Regular(ty) | ParamType::Slice { element: ty, .. } => {
                used.extend(ty.ref_lifetime.iter().cloned());
                used.extend(ty.type_args.iter().cloned());
            }
            ParamType::ImplTrait {
                type_args,
                lifetime_bounds,
                ref_lifetime,
                ..
            } => {
                used.extend(type_args.iter().cloned());
                used.extend(lifetime_bounds.iter().cloned());
                used.extend(ref_lifetime.iter().cloned());
            }
        }
    }
    if let Some(ty) = return_type(ret) {
        used.extend(ty.ref_lifetime.iter().cloned());
        used.extend(ty.type_args.iter().cloned());
    }
    let mut fresh = || {
        let name = (0..)
            .map(|index| format!("__ffier_arg_{index}"))
            .find(|name| !used.contains(name))
            .unwrap();
        used.insert(name.clone());
        generics.push(name.clone());
        name
    };

    for param in params.iter_mut() {
        if let ParamType::Regular(ty) = &mut param.param_type
            && borrowed_handle(ty)
        {
            if anonymous(ty.ref_lifetime.as_deref()) {
                ty.ref_lifetime = Some(fresh());
            }
            if ty.type_args.is_empty() {
                let count = library
                    .type_entry(&ty.type_name)
                    .unwrap()
                    .lifetime_params
                    .len();
                ty.type_args.resize(count, "_".into());
            }
            for argument in &mut ty.type_args {
                if argument == "_" {
                    *argument = fresh();
                }
            }
        }
    }

    if let Some(output) = return_type_mut(ret)
        && output.type_args.is_empty()
        && let Some(entry) = library.type_entry(&output.type_name)
    {
        output
            .type_args
            .resize(entry.lifetime_params.len(), "_".into());
    }
    let Some(output) = return_type(ret) else {
        return;
    };
    if !(output.ref_kind != RefKind::None && anonymous(output.ref_lifetime.as_deref())
        || output.type_args.iter().any(|argument| argument == "_"))
    {
        return;
    }

    let output_lifetime = if matches!(receiver, Receiver::Ref | Receiver::Mut) {
        if anonymous(receiver_lifetime.as_deref()) {
            *receiver_lifetime = Some(fresh());
        }
        receiver_lifetime.clone().unwrap()
    } else {
        let mut candidates = HashSet::new();
        let mut anonymous_count = 0;
        let mut record = |lifetime: Option<&str>| {
            if anonymous(lifetime) {
                anonymous_count += 1;
            } else {
                candidates.insert(lifetime.unwrap().to_string());
            }
        };
        for param in params {
            match &param.param_type {
                ParamType::Regular(ty) | ParamType::Slice { element: ty, .. } => {
                    if matches!(&param.param_type, ParamType::Slice { .. }) {
                        record(None); // The schema does not name the outer slice reference.
                    }
                    if ty.ref_kind != RefKind::None {
                        record(ty.ref_lifetime.as_deref());
                    }
                    for argument in &ty.type_args {
                        record(Some(argument));
                    }
                }
                ParamType::ImplTrait {
                    ref_kind,
                    ref_lifetime,
                    ..
                } if *ref_kind != RefKind::None => record(ref_lifetime.as_deref()),
                // Lifetimes in impl Trait bounds do not participate in elision.
                ParamType::ImplTrait { .. } => {}
            }
        }
        assert!(
            anonymous_count == 0 && candidates.len() == 1,
            "cannot preserve ambiguous output lifetime in generated Rust bindings"
        );
        candidates.into_iter().next().unwrap()
    };
    let output = return_type_mut(ret).unwrap();
    if output.ref_kind != RefKind::None && anonymous(output.ref_lifetime.as_deref()) {
        output.ref_lifetime = Some(output_lifetime.clone());
    }
    for argument in &mut output.type_args {
        if argument == "_" {
            *argument = output_lifetime.clone();
        }
    }
}

fn anonymous(lifetime: Option<&str>) -> bool {
    matches!(lifetime, None | Some("_"))
}

fn return_type(ret: &Return) -> Option<&TypeRef> {
    match ret {
        Return::Value(ty) | Return::OptionalValue(ty) | Return::Result { ok: Some(ty), .. } => {
            Some(ty)
        }
        _ => None,
    }
}

fn return_type_mut(ret: &mut Return) -> Option<&mut TypeRef> {
    match ret {
        Return::Value(ty) | Return::OptionalValue(ty) | Return::Result { ok: Some(ty), .. } => {
            Some(ty)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn library() -> Library {
        Library::from_json(
            &json!({
                "prefix": "test", "exported_types": [], "errors": [],
                "trait_impls": [], "traits": [],
                "type_registry": {
                    "Widget": {"kind": {"handle": {"c_name": "Widget"}}, "type_tag": 1},
                    "View": {"kind": {"handle": {"c_name": "View"}}, "type_tag": 2,
                        "lifetime_params": [{"name": "owner", "variance": "covariant"}]}
                }
            })
            .to_string(),
        )
        .unwrap()
    }

    fn method_with(params: Value, ret: Value) -> Method {
        serde_json::from_value(json!({
            "name": "test", "ffi_name": "test", "receiver": "none",
            "params": params, "ret": ret
        }))
        .unwrap()
    }

    #[test]
    fn input_only_names_reference_without_changing_schema() {
        let original = method_with(
            json!([{"name": "widget", "param_kind": "regular", "type": "Widget", "ref_kind": "shared"}]),
            json!({"return_kind": "void"}),
        );
        let normalized = method(&original, &library());
        assert_eq!(normalized.method_lifetimes, ["__ffier_arg_0"]);
        assert_eq!(
            normalized.params[0].param_type,
            ParamType::Regular(TypeRef {
                type_name: "Widget".into(),
                ref_kind: RefKind::Shared,
                ref_lifetime: Some("__ffier_arg_0".into()),
                type_args: vec![],
                optional: false,
                owned: false,
            })
        );
        assert!(original.method_lifetimes.is_empty());
    }

    #[test]
    fn sole_handle_input_supplies_elided_string_output() {
        let original = method_with(
            json!([{"name": "widget", "param_kind": "regular", "type": "Widget", "ref_kind": "shared"}]),
            json!({"return_kind": "value", "type": "str", "ref_kind": "shared"}),
        );
        let normalized = method(&original, &library());
        assert_eq!(
            return_type(&normalized.ret)
                .unwrap()
                .ref_lifetime
                .as_deref(),
            Some("__ffier_arg_0")
        );
    }

    #[test]
    fn free_function_names_implicit_output_type_lifetime() {
        let original: FreeFunction = serde_json::from_value(json!({
            "name": "view", "ffi_name": "test_view",
            "params": [{"name": "widget", "param_kind": "regular", "type": "Widget", "ref_kind": "shared"}],
            "ret": {"return_kind": "value", "type": "View"}
        })).unwrap();
        let normalized = free_function(&original, &library());
        assert_eq!(
            return_type(&normalized.ret).unwrap().type_args,
            ["__ffier_arg_0"]
        );
        assert!(return_type(&original.ret).unwrap().type_args.is_empty());
    }

    #[test]
    fn callback_bounds_do_not_change_output_elision() {
        let mut original = method_with(
            json!([
                {"name": "widget", "param_kind": "regular", "type": "Widget", "ref_kind": "shared"},
                {"name": "callback", "param_kind": "impl_trait", "trait_name": "Callback", "type_args": ["other"], "lifetime_bounds": ["other"]}
            ]),
            json!({"return_kind": "value", "type": "str", "ref_kind": "shared"}),
        );
        original.method_lifetimes.push("other".into());
        let normalized = method(&original, &library());
        assert_eq!(
            return_type(&normalized.ret)
                .unwrap()
                .ref_lifetime
                .as_deref(),
            Some("__ffier_arg_0")
        );
    }

    #[test]
    fn preserves_named_type_arguments_and_names_anonymous_arguments() {
        let mut original = method_with(
            json!([
                {"name": "view", "param_kind": "regular", "type": "View", "ref_kind": "mut", "type_args": ["owner"]},
                {"name": "other", "param_kind": "regular", "type": "View", "ref_kind": "shared", "type_args": ["_"]},
                {"name": "implicit", "param_kind": "regular", "type": "View", "ref_kind": "shared"}
            ]),
            json!({"return_kind": "void"}),
        );
        original.method_lifetimes.push("__ffier_arg_0".into());
        let normalized = method(&original, &library());
        let ParamType::Regular(first) = &normalized.params[0].param_type else {
            panic!()
        };
        assert_eq!(first.type_args, ["owner"]);
        assert_eq!(first.ref_lifetime.as_deref(), Some("__ffier_arg_1"));
        for param in &normalized.params[1..] {
            let ParamType::Regular(ty) = &param.param_type else {
                panic!()
            };
            assert_eq!(ty.type_args.len(), 1);
            assert!(ty.type_args[0].starts_with("__ffier_arg_"));
        }
    }

    #[test]
    fn receiver_wins_over_unrelated_named_input() {
        let mut original = method_with(
            json!([{"name": "widget", "param_kind": "regular", "type": "Widget", "ref_kind": "shared", "ref_lifetime": "other"}]),
            json!({"return_kind": "value", "type": "Widget", "ref_kind": "shared"}),
        );
        original.receiver = Receiver::Ref;
        original.method_lifetimes.push("other".into());
        let normalized = method(&original, &library());
        assert_eq!(
            normalized.receiver_lifetime.as_deref(),
            Some("__ffier_arg_0")
        );
        assert_eq!(
            return_type(&normalized.ret).unwrap().ref_lifetime,
            normalized.receiver_lifetime
        );
    }

    #[test]
    fn input_bounds_constrain_type_without_constraining_each_lifetime() {
        let original = method_with(
            json!([
                {"name": "view", "param_kind": "regular", "type": "View", "ref_kind": "mut", "ref_lifetime": "borrow", "type_args": ["owner"]},
                {"name": "duplicate", "param_kind": "regular", "type": "View", "ref_kind": "shared", "ref_lifetime": "borrow", "type_args": ["owner"]},
                {"name": "widget", "param_kind": "regular", "type": "Widget", "ref_kind": "shared", "ref_lifetime": "borrow"},
                {"name": "owned", "param_kind": "regular", "type": "View", "type_args": ["owner"]}
            ]),
            json!({"return_kind": "void"}),
        );
        assert_eq!(
            input_bounds(&original.params, &library()),
            " where View<'owner>: 'borrow"
        );
    }

    #[test]
    fn input_bounds_omit_non_generic_handles() {
        let original = method_with(
            json!([{"name": "widget", "param_kind": "regular", "type": "Widget", "ref_kind": "shared"}]),
            json!({"return_kind": "void"}),
        );
        assert!(input_bounds(&original.params, &library()).is_empty());
    }
}
