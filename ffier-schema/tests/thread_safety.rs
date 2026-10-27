use ffier_schema::{Library, ThreadSafety};
use serde_json::json;

#[test]
fn thread_flags_round_trip_on_all_carriers() {
    let paths = ["/exported_types/0", "/traits/0"];
    for (expected, send, sync) in [
        (json!({}), false, false),
        (json!({"send": true}), true, false),
        (json!({"sync": true}), false, true),
        (json!({"send": true, "sync": true}), true, true),
    ] {
        let flags = ThreadSafety { send, sync };
        assert_eq!(
            serde_json::from_value::<ThreadSafety>(expected.clone()).unwrap(),
            flags
        );
        let mut schema = json!({
            "prefix": "ft", "type_registry": {}, "errors": [], "trait_impls": [],
            "exported_types": [{"name": "Widget", "destroy_ffi_name": "ft_widget_destroy",
                "is_builder_type": false, "methods": []}],
            "traits": [{"name": "Backend", "destroy_ffi_name": "ft_backend_destroy",
                "type_tag_constant": "FT_BACKEND_TYPE_TAG", "vtable_struct_c_name": "FtBackendVtable",
                "wrapper_c_name": "FtVtableBackend", "vtable_struct_name": "BackendVtable",
                "wrapper_name": "VtableBackend", "methods": [], "own_method_count": 0, "num_slots": 0}]
        });
        if send || sync {
            for path in paths {
                schema.pointer_mut(path).unwrap()["thread_safety"] = expected.clone();
            }
        }
        let library = Library::from_json(&schema.to_string()).unwrap();
        let output = serde_json::to_value(&library).unwrap();
        for path in paths {
            assert_eq!(
                output.pointer(path).unwrap().get("thread_safety"),
                (send || sync).then_some(&expected)
            );
        }
        let round_trip = Library::from_json(&library.to_json()).unwrap();
        assert_eq!(serde_json::to_value(round_trip).unwrap(), output);
    }
}
