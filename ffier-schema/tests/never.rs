use ffier_schema::{Library, Return};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn never_returns_preserve_their_semantics_in_json_and_rust() {
    for (ret, encoded, rust) in [
        (Return::Never, json!({"return_kind": "never"}), "!"),
        (
            Return::ResultNever {
                err_type: "RunError".into(),
            },
            json!({"return_kind": "result_never", "err_type": "RunError"}),
            "Result<core::convert::Infallible, RunError>",
        ),
    ] {
        assert_eq!(serde_json::to_value(&ret).unwrap(), encoded);
        assert_eq!(serde_json::from_value::<Return>(encoded).unwrap(), ret);
        assert_eq!(ret.to_rust_type(&BTreeMap::new()), rust);
        assert!(!ret.is_builder_self(&BTreeMap::new()));
    }
}

#[test]
fn result_never_keeps_its_error_and_result_framework_types() {
    let mut library: Library = serde_json::from_value(json!({
        "prefix": "test",
        "type_registry": {
            "RunError": {"kind": {"error": {"c_name": "TestRunError"}}, "type_tag": 2},
            "Error": {"kind": {"trait": {"c_name": "TestError"}}, "bless": "error_trait"},
            "Result": {"kind": {"primitive": {"c_type": "TestResult"}}, "bless": "result"},
            "Unused": {"kind": {"primitive": {"c_type": "bool"}}}
        },
        "exported_types": [], "errors": [], "traits": [], "trait_impls": [],
        "free_functions": [{
            "name": "run", "ffi_name": "test_run", "params": [],
            "ret": {"return_kind": "result_never", "err_type": "RunError"}
        }]
    }))
    .unwrap();
    library.prune_unreferenced_types();
    assert_eq!(
        library
            .type_registry
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["Error", "Result", "RunError"]
    );
}
