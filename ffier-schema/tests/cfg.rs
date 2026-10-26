use ffier_schema::Cfg;
use serde_json::json;

#[test]
fn predicates_round_trip_as_objects() {
    for (json, expected) in [
        (json!({"kind": "windows"}), "windows"),
        (json!({"kind": "unix"}), "unix"),
        (
            json!({"kind": "target_os", "value": "windows"}),
            "target_os = \"windows\"",
        ),
        (
            json!({"kind": "feature", "value": "files"}),
            "feature = \"files\"",
        ),
        (
            json!({"kind": "all", "value": [
                {"kind": "unix"},
                {"kind": "feature", "value": "files"}
            ]}),
            "all(unix, feature = \"files\")",
        ),
        (
            json!({"kind": "not", "value": {"kind": "any", "value": [
                {"kind": "windows"},
                {"kind": "target_os", "value": "macos"}
            ]}}),
            "not(any(windows, target_os = \"macos\"))",
        ),
        (json!({"kind": "all", "value": []}), "all()"),
        (json!({"kind": "any", "value": []}), "any()"),
    ] {
        let cfg: Cfg = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(cfg.to_string(), expected);
        assert_eq!(serde_json::to_value(cfg).unwrap(), json);
    }
}

#[test]
fn rejects_unsupported_predicates_and_invalid_shapes() {
    for json in [
        json!("unix"),
        json!({}),
        json!({"kind": "target_arch", "value": "x86_64"}),
        json!({"kind": "target_family", "value": "unix"}),
        json!({"kind": "target_pointer_width", "value": "64"}),
        json!({"kind": "custom"}),
        json!({"kind": "feature"}),
        json!({"kind": "target_os", "value": 1}),
        json!({"kind": "all", "value": {"kind": "unix"}}),
        json!({"kind": "not", "value": [{"kind": "unix"}]}),
        json!({"kind": "unix", "extra": true}),
        json!({"kind": "all", "value": [{"kind": "custom"}]}),
    ] {
        assert!(
            serde_json::from_value::<Cfg>(json.clone()).is_err(),
            "{json}"
        );
    }
}

#[test]
fn rejects_duplicate_predicate_fields() {
    for text in [
        r#"{"kind":"unix","kind":"windows"}"#,
        r#"{"kind":"feature","value":"files","value":"logging"}"#,
    ] {
        assert!(serde_json::from_str::<Cfg>(text).is_err());
    }
}

#[test]
fn renders_escaped_feature_values() {
    assert_eq!(
        Cfg::Feature("files\"\\\n".into()).to_string(),
        "feature = \"files\\\"\\\\\\n\""
    );
}
