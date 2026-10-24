use ffier_schema::Cfg;
use std::collections::HashMap;

fn contains_feature(cfg: &Cfg) -> bool {
    match cfg {
        Cfg::Value { key, .. } => key == "feature",
        Cfg::All(items) | Cfg::Any(items) => items.iter().any(contains_feature),
        Cfg::Not(item) => contains_feature(item),
        Cfg::Flag(_) => false,
    }
}

// Compute an upper bound on availability when the producer's Cargo features
// are unknown. Negation swaps upper/lower bounds; platform predicates remain
// intact even inside nested expressions such as not(any(feature, windows)).
fn relax_features(cfg: &Cfg, upper: bool) -> Cfg {
    match cfg {
        Cfg::Value { key, .. } if key == "feature" => {
            if upper {
                Cfg::All(vec![])
            } else {
                Cfg::Any(vec![])
            }
        }
        Cfg::All(items) => Cfg::All(items.iter().map(|x| relax_features(x, upper)).collect()),
        Cfg::Any(items) => Cfg::Any(items.iter().map(|x| relax_features(x, upper)).collect()),
        Cfg::Not(item) => Cfg::Not(Box::new(relax_features(item, !upper))),
        _ => cfg.clone(),
    }
}

pub fn client_cfg(cfg: &Cfg) -> String {
    if !contains_feature(cfg) {
        return cfg.to_string();
    }
    let relaxed = relax_features(cfg, true);
    format!("any(all(not(feature = \"strong\"), {relaxed}), all(feature = \"strong\", {cfg}))")
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
            relax_features(&predicate, true).to_string(),
            "not(any(any(), target_os = \"windows\"))"
        );
    }
}
