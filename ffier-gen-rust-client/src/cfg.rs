use ffier_schema::Cfg;

/// Render an availability predicate for generated Rust clients.
pub fn client_cfg(cfg: &Cfg) -> String {
    // TODO: Support explicit assumptions for non-additive features that change
    // signatures, like C header feature options in docs/conditional-compilation.md.
    match cfg {
        Cfg::Feature(name) => format!("any(not(feature = \"ffier-strong\"), feature = {name:?})"),
        Cfg::All(items) | Cfg::Any(items) => {
            let operator = if matches!(cfg, Cfg::All(_)) {
                "all"
            } else {
                "any"
            };
            let items = items.iter().map(client_cfg).collect::<Vec<_>>().join(", ");
            format!("{operator}({items})")
        }
        Cfg::Not(_) | Cfg::Windows | Cfg::Unix | Cfg::TargetOs(_) => cfg.to_string(),
    }
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
    fn feature_predicates_keep_optional_api_available_for_dynamic_lookup() {
        let feature = Cfg::Feature("optional-entry".into());
        assert_eq!(
            client_cfg(&feature),
            "any(not(feature = \"ffier-strong\"), feature = \"optional-entry\")"
        );
        assert_eq!(
            client_cfg(&Cfg::Not(Box::new(feature))),
            "not(feature = \"optional-entry\")"
        );
    }

    #[test]
    fn platforms_and_constants_apply_to_both_linking_modes() {
        for (cfg, expected) in [
            (Cfg::Windows, "windows"),
            (Cfg::Unix, "unix"),
            (Cfg::TargetOs("linux".into()), "target_os = \"linux\""),
            (Cfg::All(vec![]), "all()"),
            (Cfg::Any(vec![]), "any()"),
        ] {
            assert_eq!(client_cfg(&cfg), expected);
        }
    }

    #[test]
    fn mixed_predicates_relax_features_without_relaxing_platforms() {
        let cfg = Cfg::All(vec![Cfg::Windows, Cfg::Feature("files".into())]);
        assert_eq!(
            client_cfg(&cfg),
            "all(windows, any(not(feature = \"ffier-strong\"), feature = \"files\"))"
        );
        assert_eq!(
            client_cfg(&Cfg::Any(vec![Cfg::Unix, cfg])),
            "any(unix, all(windows, any(not(feature = \"ffier-strong\"), feature = \"files\")))"
        );
    }

    #[test]
    fn negated_predicates_remain_unchanged() {
        let cfg = Cfg::Not(Box::new(Cfg::Any(vec![
            Cfg::Feature("files".into()),
            Cfg::Windows,
        ])));
        assert_eq!(client_cfg(&cfg), "not(any(feature = \"files\", windows))");

        let nested = Cfg::Not(Box::new(Cfg::Any(vec![
            Cfg::Unix,
            Cfg::Not(Box::new(Cfg::All(vec![
                Cfg::Feature("files".into()),
                Cfg::Windows,
            ]))),
        ])));
        assert_eq!(
            client_cfg(&nested),
            "not(any(unix, not(all(feature = \"files\", windows))))"
        );
    }

    #[test]
    fn combines_owner_and_method_predicates_without_losing_either() {
        let owner = Cfg::Feature("owner".into());
        let method = Cfg::Windows;
        assert_eq!(combine(None, None), None);
        assert_eq!(combine(Some(&owner), None), Some(owner.clone()));
        assert_eq!(combine(None, Some(&method)), Some(method.clone()));
        assert_eq!(
            combine(Some(&owner), Some(&method)),
            Some(Cfg::All(vec![owner, method]))
        );
    }
}
