use ffier_schema::{
    ExportedType, ImplementableTrait, Library, Param, ParamType, RefKind, ThreadSafety,
};
use serde_json::json;

#[test]
fn independent_flags_serialize_alone() {
    // Send-only: sync omitted.
    assert_eq!(
        serde_json::to_value(ThreadSafety {
            send: true,
            sync: false
        })
        .unwrap(),
        json!({"send": true})
    );
    // Sync-only: send omitted.
    assert_eq!(
        serde_json::to_value(ThreadSafety {
            send: false,
            sync: true
        })
        .unwrap(),
        json!({"sync": true})
    );
    // Both flags serialize independently.
    assert_eq!(
        serde_json::to_value(ThreadSafety {
            send: true,
            sync: true
        })
        .unwrap(),
        json!({"send": true, "sync": true})
    );
}

#[test]
fn empty_thread_safety_serializes_to_nothing() {
    let empty = ThreadSafety::default();
    assert!(empty.is_empty());
    assert_eq!(serde_json::to_value(empty).unwrap(), json!({}));
}

#[test]
fn omitted_false_flags_deserialize_to_false() {
    // Absent keys default to false on every carrier.
    let ts: ThreadSafety = serde_json::from_value(json!({})).unwrap();
    assert_eq!(ts, ThreadSafety::default());
    assert!(!ts.send && !ts.sync);

    let ts: ThreadSafety = serde_json::from_value(json!({"send": true})).unwrap();
    assert!(ts.send && !ts.sync);

    let ts: ThreadSafety = serde_json::from_value(json!({"sync": true})).unwrap();
    assert!(!ts.send && ts.sync);
}

fn exported_type(thread_safety: ThreadSafety) -> ExportedType {
    ExportedType {
        name: "Widget".to_string(),
        destroy_ffi_name: "ft_widget_destroy".to_string(),
        is_builder_type: false,
        thread_safety,
        methods: vec![],
        cfg: None,
    }
}

fn implementable_trait(thread_safety: ThreadSafety) -> ImplementableTrait {
    ImplementableTrait {
        name: "Fruit".to_string(),
        destroy_ffi_name: "ft_fruit_destroy".to_string(),
        type_tag_constant: "FT_FRUIT_TYPE_TAG".to_string(),
        vtable_struct_c_name: "FtFruitVtable".to_string(),
        wrapper_c_name: "FtVtableFruit".to_string(),
        vtable_struct_name: "FruitVtable".to_string(),
        wrapper_name: "VtableFruit".to_string(),
        thread_safety,
        methods: vec![],
        own_method_count: 0,
        num_slots: 0,
        no_vtable: false,
        cfg: None,
    }
}

fn impl_trait_param(thread_safety: ThreadSafety) -> Param {
    Param {
        name: "backend".to_string(),
        param_type: ParamType::ImplTrait {
            trait_name: "Fruit".to_string(),
            type_args: vec![],
            lifetime_bounds: vec![],
            ref_kind: RefKind::None,
            ref_lifetime: None,
            thread_safety,
        },
    }
}

#[test]
fn carriers_omit_empty_thread_safety_and_keep_set_flags() {
    // Empty: key absent from all three carriers.
    let json = serde_json::to_value(exported_type(ThreadSafety::default())).unwrap();
    assert!(json.get("thread_safety").is_none());
    let json = serde_json::to_value(implementable_trait(ThreadSafety::default())).unwrap();
    assert!(json.get("thread_safety").is_none());
    let json = serde_json::to_value(impl_trait_param(ThreadSafety::default())).unwrap();
    assert!(json.get("thread_safety").is_none());

    // Set: key present with independent flags on all three carriers.
    let send_only = ThreadSafety {
        send: true,
        sync: false,
    };
    let json = serde_json::to_value(exported_type(send_only)).unwrap();
    assert_eq!(json["thread_safety"], json!({"send": true}));
    let json = serde_json::to_value(implementable_trait(send_only)).unwrap();
    assert_eq!(json["thread_safety"], json!({"send": true}));
    let json = serde_json::to_value(impl_trait_param(send_only)).unwrap();
    assert_eq!(json["thread_safety"], json!({"send": true}));
}

#[test]
fn library_json_round_trips_thread_safety_on_all_carriers() {
    let mut lib = Library {
        prefix: "ft".to_string(),
        primitives_prefix: None,
        type_registry: Default::default(),
        exported_types: vec![exported_type(ThreadSafety {
            send: true,
            sync: false,
        })],
        errors: vec![],
        enum_constants: vec![],
        bitflags_constants: vec![],
        free_functions: vec![],
        traits: vec![implementable_trait(ThreadSafety {
            send: true,
            sync: true,
        })],
        trait_impls: vec![],
    };

    let parsed = Library::from_json(&lib.to_json()).unwrap();
    assert_eq!(
        parsed.exported_types[0].thread_safety,
        ThreadSafety {
            send: true,
            sync: false
        }
    );
    assert_eq!(
        parsed.traits[0].thread_safety,
        ThreadSafety {
            send: true,
            sync: true
        }
    );
    let ParamType::ImplTrait { thread_safety, .. } = &impl_trait_param(ThreadSafety {
        send: false,
        sync: true,
    })
    .param_type
    else {
        unreachable!()
    };
    lib.exported_types[0].methods = vec![];
    assert_eq!(
        *thread_safety,
        ThreadSafety {
            send: false,
            sync: true
        }
    );
}

#[test]
fn rust_bounds_use_absolute_core_marker_paths() {
    let none = ThreadSafety::default();
    assert_eq!(none.rust_bounds().collect::<Vec<_>>(), Vec::<&str>::new());

    let send_only = ThreadSafety {
        send: true,
        sync: false,
    };
    assert_eq!(
        send_only.rust_bounds().collect::<Vec<_>>(),
        vec!["::core::marker::Send"]
    );

    let sync_only = ThreadSafety {
        send: false,
        sync: true,
    };
    assert_eq!(
        sync_only.rust_bounds().collect::<Vec<_>>(),
        vec!["::core::marker::Sync"]
    );

    let both = ThreadSafety {
        send: true,
        sync: true,
    };
    assert_eq!(
        both.rust_bounds().collect::<Vec<_>>(),
        vec!["::core::marker::Send", "::core::marker::Sync"]
    );
}
