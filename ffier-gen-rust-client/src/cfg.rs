use ffier_schema::Cfg;

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
