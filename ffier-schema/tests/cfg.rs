use ffier_schema::Cfg;
use serde_json::json;

#[test]
fn predicates_roundtrip_without_parsing_rust_strings() {
    for (value, rust) in [
        (json!({"target_os": "windows"}), "target_os = \"windows\""),
        (json!({"unix": true}), "unix"),
        (
            json!({"all": [{"unix": true}, {"feature": "files"}]}),
            "all(unix, feature = \"files\")",
        ),
        (
            json!({"not": {"any": [{"windows": true}, {"target_os": "macos"}]}}),
            "not(any(windows, target_os = \"macos\"))",
        ),
        (json!({"all": []}), "all()"),
        (json!({"any": []}), "any()"),
        (
            json!({"custom_key": "a\"b\\c"}),
            "custom_key = \"a\\\"b\\\\c\"",
        ),
    ] {
        let cfg: Cfg = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(cfg.to_string(), rust);
        assert_eq!(serde_json::to_value(&cfg).unwrap(), value);
        assert_eq!(
            serde_json::from_str::<Cfg>(&serde_json::to_string(&cfg).unwrap()).unwrap(),
            cfg
        );
    }
}

#[test]
fn malformed_predicate_objects_are_rejected() {
    for value in [
        json!("unix"),
        json!({}),
        json!({"unix": false}),
        json!({"target_os": 1}),
        json!({"unix": true, "windows": true}),
        json!({"all": {"unix": true}}),
        json!({"not": []}),
        json!({"not": null}),
        json!({"a b": true}),
        json!({"1key": "value"}),
    ] {
        assert!(
            serde_json::from_value::<Cfg>(value.clone()).is_err(),
            "{value}"
        );
    }
    assert!(serde_json::from_str::<Cfg>(r#"{"unix":true,"unix":true}"#).is_err());
}
