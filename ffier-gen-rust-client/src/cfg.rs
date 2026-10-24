use ffier_schema::Cfg;
use std::collections::{HashMap, HashSet};

fn contains_optional_feature(cfg: &Cfg, signature_features: &HashSet<String>) -> bool {
    match cfg {
        Cfg::Value { key, value } => key == "feature" && !signature_features.contains(value),
        Cfg::All(items) | Cfg::Any(items) => items
            .iter()
            .any(|item| contains_optional_feature(item, signature_features)),
        Cfg::Not(item) => contains_optional_feature(item, signature_features),
        Cfg::Flag(_) => false,
    }
}

// Compute an upper bound on availability when the producer's Cargo features
// are unknown. Negation swaps upper/lower bounds; platform predicates remain
// intact even inside nested expressions such as not(any(feature, windows)).
fn relax_features(cfg: &Cfg, upper: bool, signature_features: &HashSet<String>) -> Cfg {
    match cfg {
        Cfg::Value { key, value } if key == "feature" && !signature_features.contains(value) => {
            if upper {
                Cfg::All(vec![])
            } else {
                Cfg::Any(vec![])
            }
        }
        Cfg::All(items) => Cfg::All(
            items
                .iter()
                .map(|x| relax_features(x, upper, signature_features))
                .collect(),
        ),
        Cfg::Any(items) => Cfg::Any(
            items
                .iter()
                .map(|x| relax_features(x, upper, signature_features))
                .collect(),
        ),
        Cfg::Not(item) => Cfg::Not(Box::new(relax_features(item, !upper, signature_features))),
        _ => cfg.clone(),
    }
}

pub fn client_cfg(cfg: &Cfg) -> String {
    client_cfg_for_signature(cfg, &HashSet::new())
}

pub fn client_cfg_for_signature(cfg: &Cfg, signature_features: &HashSet<String>) -> String {
    if !contains_optional_feature(cfg, signature_features) {
        return cfg.to_string();
    }
    let relaxed = relax_features(cfg, true, signature_features);
    format!("any(all(not(feature = \"strong\"), {relaxed}), all(feature = \"strong\", {cfg}))")
}

fn platform_form(cfg: &Cfg, negated: bool) -> Cfg {
    match cfg {
        Cfg::Not(item) => platform_form(item, !negated),
        Cfg::All(items) | Cfg::Any(items) => {
            let items = items
                .iter()
                .map(|item| platform_form(item, negated))
                .collect();
            if matches!(cfg, Cfg::All(_)) != negated {
                Cfg::All(items)
            } else {
                Cfg::Any(items)
            }
        }
        Cfg::Flag(name) if name == "windows" => platform_form(
            &Cfg::Value {
                key: "target_os".into(),
                value: "windows".into(),
            },
            negated,
        ),
        Cfg::Value { key, value }
            if key == "target_family" && matches!(value.as_str(), "unix" | "windows") =>
        {
            platform_form(&Cfg::Flag(value.clone()), negated)
        }
        _ if negated => Cfg::Not(Box::new(cfg.clone())),
        _ => cfg.clone(),
    }
}

/// Prove disjointness without assuming custom cfg flags or multi-valued keys
/// such as target_feature and target_family are mutually exclusive.
fn disjoint_platforms(a: &Cfg, b: &Cfg) -> bool {
    match (a, b) {
        (Cfg::Any(items), _) => items.iter().all(|item| disjoint_platforms(item, b)),
        (_, Cfg::Any(items)) => items.iter().all(|item| disjoint_platforms(a, item)),
        (Cfg::All(items), _) => items.iter().any(|item| disjoint_platforms(item, b)),
        (_, Cfg::All(items)) => items.iter().any(|item| disjoint_platforms(a, item)),
        (Cfg::Not(item), other) | (other, Cfg::Not(item)) if item.as_ref() == other => true,
        (
            Cfg::Value {
                key: a_key,
                value: a_value,
            },
            Cfg::Value {
                key: b_key,
                value: b_value,
            },
        ) => {
            a_key == b_key
                && a_value != b_value
                && matches!(
                    a_key.as_str(),
                    "target_os"
                        | "target_arch"
                        | "target_pointer_width"
                        | "target_endian"
                        | "target_env"
                        | "target_vendor"
                        | "target_abi"
                )
        }
        (Cfg::Flag(flag), Cfg::Value { key, value })
        | (Cfg::Value { key, value }, Cfg::Flag(flag)) => {
            flag == "unix" && key == "target_os" && value == "windows"
        }
        _ => false,
    }
}

/// Shared conjunctions describe availability unless they constrain features
/// selecting between signatures. Keep those choices and their correlations strict.
pub fn signature_features<'a>(
    predicates: impl IntoIterator<Item = Option<&'a Cfg>>,
) -> HashSet<String> {
    fn conjuncts(cfg: &Cfg, clauses: &mut Vec<Cfg>) {
        if let Cfg::All(items) = cfg {
            for item in items {
                conjuncts(item, clauses);
            }
        } else {
            clauses.push(cfg.clone());
        }
    }
    fn atoms(cfg: &Cfg, output: &mut HashSet<(String, Option<String>)>) {
        match cfg {
            Cfg::Value { key, value } => {
                output.insert((key.clone(), Some(value.clone())));
            }
            Cfg::Flag(name) => {
                output.insert((name.clone(), None));
            }
            Cfg::All(items) | Cfg::Any(items) => {
                for item in items {
                    atoms(item, output);
                }
            }
            Cfg::Not(item) => atoms(item, output),
        }
    }
    let alternatives: Vec<Vec<Cfg>> = predicates
        .into_iter()
        .map(|cfg| {
            let mut clauses = Vec::new();
            if let Some(cfg) = cfg {
                conjuncts(cfg, &mut clauses);
            }
            clauses
        })
        .collect();
    if alternatives.len() < 2 {
        return HashSet::new();
    }
    let mut output = HashSet::new();
    let mut shared_atoms = Vec::new();
    let platforms: Vec<_> = alternatives
        .iter()
        .map(|clauses| {
            platform_form(
                &relax_features(&Cfg::All(clauses.clone()), true, &HashSet::new()),
                false,
            )
        })
        .collect();
    for (index, a) in alternatives.iter().enumerate() {
        for (other, b) in alternatives.iter().enumerate().skip(index + 1) {
            if disjoint_platforms(&platforms[index], &platforms[other]) {
                continue;
            }
            for clause in a
                .iter()
                .filter(|clause| !b.contains(clause))
                .chain(b.iter().filter(|clause| !a.contains(clause)))
            {
                atoms(clause, &mut output);
            }
            for clause in a.iter().filter(|clause| b.contains(clause)) {
                let mut clause_atoms = HashSet::new();
                atoms(clause, &mut clause_atoms);
                shared_atoms.push(clause_atoms);
            }
        }
    }
    loop {
        let previous_len = output.len();
        for clause in &shared_atoms {
            if !clause.is_disjoint(&output) {
                output.extend(clause.iter().cloned());
            }
        }
        if output.len() == previous_len {
            break;
        }
    }
    output
        .into_iter()
        .filter_map(|(key, value)| if key == "feature" { value } else { None })
        .collect()
}

pub fn combine(a: Option<&Cfg>, b: Option<&Cfg>) -> Option<Cfg> {
    match (a, b) {
        (Some(a), Some(b)) => Some(Cfg::All(vec![a.clone(), b.clone()])),
        (a, b) => a.or(b).cloned(),
    }
}

/// A foreign wrapper is needed wherever any signature using it is available.
/// Apply this to imports and both delegating impls, as well as to the methods.
pub fn foreign_types(lib: &ffier_schema::Library) -> HashMap<String, Option<Cfg>> {
    use ffier_schema::{Param, ParamType, Return, TypeKind};
    fn signature(
        params: &[Param],
        ret: &Return,
        cfg: Option<&Cfg>,
        record: &mut impl FnMut(&str, Option<&Cfg>),
    ) {
        for param in params {
            match &param.param_type {
                ParamType::Regular(ty) => record(&ty.type_name, cfg),
                ParamType::Slice { element, .. } => record(&element.type_name, cfg),
                ParamType::ImplTrait { trait_name, .. } => record(trait_name, cfg),
            }
        }
        match ret {
            Return::Value(ty) => record(&ty.type_name, cfg),
            Return::Result { ok, err_type, .. } => {
                if let Some(ty) = ok {
                    record(&ty.type_name, cfg);
                }
                record(err_type, cfg);
            }
            Return::ObjectArray { element } => record(&element.type_name, cfg),
            Return::Void => {}
        }
    }
    let mut predicates = HashMap::new();
    let mut record = |name: &str, cfg: Option<&Cfg>| {
        if !lib
            .type_entry(name)
            .is_some_and(|entry| matches!(entry.kind, TypeKind::ForeignHandle { .. }))
        {
            return;
        }
        predicates
            .entry(name.to_string())
            .and_modify(|previous: &mut Option<Cfg>| {
                *previous = match (previous.take(), cfg.cloned()) {
                    (Some(a), Some(b)) => Some(Cfg::Any(vec![a, b])),
                    _ => None,
                };
            })
            .or_insert_with(|| cfg.cloned());
    };
    for ty in &lib.exported_types {
        for method in &ty.methods {
            let cfg = combine(ty.cfg.as_ref(), method.cfg.as_ref());
            signature(&method.params, &method.ret, cfg.as_ref(), &mut record);
        }
    }
    for tr in &lib.traits {
        for method in &tr.methods {
            let cfg = combine(tr.cfg.as_ref(), method.cfg.as_ref());
            signature(&method.params, &method.ret, cfg.as_ref(), &mut record);
        }
    }
    for ti in &lib.trait_impls {
        record(&ti.struct_name, ti.cfg.as_ref());
        for method in &ti.methods {
            let cfg = combine(ti.cfg.as_ref(), method.cfg.as_ref());
            signature(&method.params, &method.ret, cfg.as_ref(), &mut record);
        }
    }
    for function in &lib.free_functions {
        signature(
            &function.params,
            &function.ret,
            function.cfg.as_ref(),
            &mut record,
        );
    }
    for error in &lib.errors {
        for variant in &error.variants {
            for (_, ty) in variant.data.typed_fields() {
                record(&ty.type_name, None);
            }
        }
    }
    for (name, entry) in &lib.type_registry {
        if matches!(entry.kind, TypeKind::ForeignHandle { .. }) {
            predicates
                .entry(name.clone())
                .or_insert_with(|| Some(Cfg::Any(vec![])));
        }
    }
    predicates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(name: &str) -> Cfg {
        Cfg::Value {
            key: "feature".into(),
            value: name.into(),
        }
    }

    #[test]
    fn common_features_enable_api_and_differing_features_select_signatures() {
        let owner = feature("owner");
        let wide = feature("wide");
        let variants = [
            Cfg::All(vec![owner.clone(), Cfg::All(vec![wide.clone()])]),
            Cfg::All(vec![owner.clone(), Cfg::Not(Box::new(wide.clone()))]),
        ];
        let features = signature_features(variants.iter().map(Some));
        assert_eq!(features, HashSet::from(["wide".into()]));
        assert_eq!(
            client_cfg_for_signature(&wide, &features),
            "feature = \"wide\""
        );
        assert_eq!(
            relax_features(&variants[0], true, &features),
            Cfg::All(vec![Cfg::All(vec![]), Cfg::All(vec![wide])])
        );
        let platforms = [
            Cfg::All(vec![owner.clone(), Cfg::Flag("unix".into())]),
            Cfg::All(vec![owner, Cfg::Flag("windows".into())]),
        ];
        assert!(signature_features(platforms.iter().map(Some)).is_empty());
        let optional_platform = [
            Cfg::All(vec![Cfg::Flag("unix".into()), feature("files")]),
            Cfg::Flag("windows".into()),
        ];
        assert!(signature_features(optional_platform.iter().map(Some)).is_empty());
        let mixed = [
            Cfg::All(vec![
                Cfg::Flag("unix".into()),
                feature("files"),
                feature("wide"),
            ]),
            Cfg::All(vec![
                Cfg::Flag("unix".into()),
                feature("files"),
                Cfg::Not(Box::new(feature("wide"))),
            ]),
            Cfg::Flag("windows".into()),
        ];
        assert_eq!(
            signature_features(mixed.iter().map(Some)),
            HashSet::from(["wide".into()])
        );
        assert!(signature_features([Some(&feature("override"))]).is_empty());
    }

    #[test]
    fn shared_predicates_preserve_correlated_signature_features() {
        let shared = Cfg::All(vec![
            feature("owner"),
            Cfg::Any(vec![feature("mode"), feature("a")]),
            Cfg::Any(vec![Cfg::Not(Box::new(feature("mode"))), feature("b")]),
        ]);
        let variants = [
            Cfg::All(vec![shared.clone(), Cfg::Not(Box::new(feature("a")))]),
            Cfg::All(vec![shared, Cfg::Not(Box::new(feature("b")))]),
        ];
        let strict = signature_features(variants.iter().map(Some));
        assert_eq!(
            strict,
            HashSet::from(["mode".into(), "a".into(), "b".into()])
        );

        fn evaluate(cfg: &Cfg, enabled: &HashSet<String>) -> bool {
            match cfg {
                Cfg::Value { key, value } if key == "feature" => enabled.contains(value),
                Cfg::All(items) => items.iter().all(|item| evaluate(item, enabled)),
                Cfg::Any(items) => items.iter().any(|item| evaluate(item, enabled)),
                Cfg::Not(item) => !evaluate(item, enabled),
                _ => panic!("expected a feature predicate"),
            }
        }
        let relaxed = variants.map(|cfg| relax_features(&cfg, true, &strict));
        for mask in 0..8 {
            let enabled = ["mode", "a", "b"]
                .into_iter()
                .enumerate()
                .filter(|(bit, _)| mask & (1 << bit) != 0)
                .map(|(_, name)| name.to_owned())
                .collect();
            assert!(
                !evaluate(&relaxed[0], &enabled) || !evaluate(&relaxed[1], &enabled),
                "relaxed signatures overlap with features {enabled:?}"
            );
        }
    }

    #[test]
    fn target_environments_do_not_select_feature_signatures() {
        let env = |value: &str| Cfg::Value {
            key: "target_env".into(),
            value: value.into(),
        };
        let variants = [Cfg::All(vec![env("gnu"), feature("files")]), env("musl")];
        assert!(signature_features(variants.iter().map(Some)).is_empty());
    }

    #[test]
    fn custom_flags_preserve_correlated_signature_features() {
        let a = Cfg::Flag("a".into());
        let b = Cfg::Flag("b".into());
        let wide = feature("wide");
        let shared = Cfg::All(vec![
            Cfg::Any(vec![Cfg::Not(Box::new(a.clone())), wide.clone()]),
            Cfg::Any(vec![
                Cfg::Not(Box::new(b.clone())),
                Cfg::Not(Box::new(wide)),
            ]),
        ]);
        let variants = [Cfg::All(vec![shared.clone(), a]), Cfg::All(vec![shared, b])];
        let strict = signature_features(variants.iter().map(Some));
        assert_eq!(strict, HashSet::from(["wide".into()]));
        for cfg in variants {
            assert_eq!(relax_features(&cfg, true, &strict), cfg);
        }
    }

    #[test]
    fn relaxing_features_keeps_platform_constraints_under_negation() {
        let windows = Cfg::Value {
            key: "target_os".into(),
            value: "windows".into(),
        };
        let feature = Cfg::Value {
            key: "feature".into(),
            value: "files".into(),
        };
        assert_eq!(client_cfg(&windows), "target_os = \"windows\"");
        let predicate = Cfg::Not(Box::new(Cfg::Any(vec![feature, windows])));
        assert_eq!(
            relax_features(&predicate, true, &HashSet::new()).to_string(),
            "not(any(any(), target_os = \"windows\"))"
        );
    }
}
