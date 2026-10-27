use ffier_gen_rust_client::generate;
use ffier_schema::{Cfg, Library};
use serde_json::{Value, json};

fn method(name: &str, cfg: Option<Cfg>, windows_param: bool, index: Option<usize>) -> Value {
    let params = if windows_param {
        vec![json!({
            "name": "resource", "param_kind": "regular", "type": "BorrowedHandle", "type_args": ["_"]
        })]
    } else {
        vec![]
    };
    let mut method = json!({
        "name": name, "ffi_name": format!("probe_widget_{name}"),
        "receiver": "ref", "params": params,
        "ret": {"return_kind": "value", "type": "i32"}, "cfg": cfg
    });
    if let Some(index) = index {
        method["trait_definition"] = json!({"index": index, "has_default": false});
    }
    method
}

fn callback(name: &str, methods: Vec<Value>, cfg: Option<Cfg>, num_slots: usize) -> Value {
    json!({
        "name": name, "destroy_ffi_name": format!("probe_{name}_destroy"),
        "type_tag_constant": format!("PROBE_{name}_TYPE_TAG"),
        "vtable_struct_c_name": format!("Probe{name}Vtable"),
        "wrapper_c_name": format!("ProbeVtable{name}"),
        "vtable_struct_name": format!("{name}Vtable"), "wrapper_name": format!("Vtable{name}"),
        "own_method_count": methods.len(), "methods": methods, "num_slots": num_slots, "cfg": cfg
    })
}

fn schema() -> Library {
    let optional = Cfg::Feature("optional".into());
    let mut windows_owner_method = method("owner_call", Some(optional.clone()), true, None);
    windows_owner_method["ffi_name"] = json!("probe_windows_owner_call");
    let mut windows_callback_method = method("owner_hook", None, true, Some(0));
    windows_callback_method["ffi_name"] = json!("probe_windows_callback_hook");
    let mut callback_methods = vec![
        method("windows_hook", Some(Cfg::Windows), true, Some(0)),
        method("optional_hook", Some(optional.clone()), false, Some(1)),
        method("last_hook", None, false, Some(3)),
    ];
    callback_methods[0]["trait_definition"]["has_default"] = json!(true);
    let impl_methods: Vec<_> = callback_methods
        .iter()
        .filter(|method| method["name"] != "windows_hook")
        .cloned()
        .map(|mut method| {
            method.as_object_mut().unwrap().remove("trait_definition");
            let name = method["name"].as_str().unwrap();
            method["ffi_name"] = json!(format!("probe_widget_callback_{name}"));
            method
        })
        .collect();
    serde_json::from_value(json!({
        "prefix": "probe",
        "type_registry": {
            "i32": {"kind": {"primitive": {"c_type": "int32_t"}}},
            "*mut core::ffi::c_void": {"kind": {"primitive": {"c_type": "void*"}}},
            "BorrowedHandle": {
                "kind": {"alias": {"alias_of": "*mut core::ffi::c_void"}},
                "bless": "borrowed_handle", "lifetime_params": [{"name": "handle"}]
            },
            "Widget": {"kind": {"handle": {"c_name": "ProbeWidget"}}, "type_tag": 1},
            "WindowsOwner": {"kind": {"handle": {"c_name": "ProbeWindowsOwner"}}, "type_tag": 2},
            "Callback": {"kind": {"trait": {"c_name": "ProbeCallback"}}, "type_tag": 3},
            "WindowsCallback": {"kind": {"trait": {"c_name": "ProbeWindowsCallback"}}, "type_tag": 4}
        },
        "exported_types": [
            {"name": "Widget", "destroy_ffi_name": "probe_widget_destroy", "is_builder_type": false,
             "methods": [
                method("windows_only", Some(Cfg::Windows), true, None),
                method("optional", Some(optional.clone()), false, None),
                method("mixed", Some(Cfg::All(vec![Cfg::Windows, optional])), true, None)
             ]},
            {"name": "WindowsOwner", "destroy_ffi_name": "probe_windows_owner_destroy", "is_builder_type": false,
             "cfg": Cfg::Windows, "methods": [windows_owner_method]}
        ],
        "errors": [],
        "traits": [
            callback("Callback", callback_methods, None, 4),
            callback("WindowsCallback", vec![windows_callback_method], Some(Cfg::Windows), 1)
        ],
        "trait_impls": [{"trait_name": "Callback", "struct_name": "Widget", "methods": impl_methods}],
        "free_functions": [{
            "name": "windows_free", "ffi_name": "probe_windows_free",
            "cfg": Cfg::TargetOs("windows".into()),
            "params": [{"name": "resource", "param_kind": "regular", "type": "BorrowedHandle", "type_args": ["_"]}],
            "ret": {"return_kind": "void"}
        }]
    }))
    .unwrap()
}

#[test]
fn platform_wrappers_keep_platform_guards_in_dynamic_clients() {
    let source = generate(&schema());
    assert!(source.contains("#[cfg(windows)]\npub struct WindowsOwner"));
    assert!(source.contains("#[cfg(windows)]\npub trait WindowsCallback"));
}
