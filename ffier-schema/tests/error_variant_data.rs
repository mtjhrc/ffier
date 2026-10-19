use ffier_schema::{ErrorField, ErrorVariantData, Library, RefKind, TypeRef};
use serde_json::json;

fn typed_field(name: &str) -> ErrorField {
    ErrorField::Typed(TypeRef {
        type_name: name.to_string(),
        ref_kind: RefKind::None,
        ref_lifetime: None,
        type_args: vec![],
        optional: false,
        owned: false,
    })
}

#[test]
fn fieldless_and_empty_tuple_preserve_different_declarations() {
    for (data, json) in [
        (ErrorVariantData::Fieldless, json!({"kind": "fieldless"})),
        (
            ErrorVariantData::Tuple(vec![]),
            json!({"kind": "tuple", "fields": []}),
        ),
    ] {
        assert!(data.fields().is_empty());
        assert_eq!(serde_json::to_value(&data).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<ErrorVariantData>(json).unwrap(),
            data
        );
    }
}

#[test]
fn unit_type_payload_is_one_field_not_an_empty_tuple() {
    let data = ErrorVariantData::Tuple(vec![typed_field("()")]);
    assert_eq!(data.fields().len(), 1);
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["fields"][0]["type_ref"]["type"], "()");
    assert_eq!(
        serde_json::from_value::<ErrorVariantData>(json).unwrap(),
        data
    );
}

#[test]
fn opaque_fields_keep_their_positions_between_typed_fields() {
    let data = ErrorVariantData::Tuple(vec![
        typed_field("i32"),
        ErrorField::Opaque,
        typed_field("u64"),
    ]);
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["fields"][1], json!({"kind": "opaque"}));
    let decoded = serde_json::from_value::<ErrorVariantData>(json).unwrap();
    assert_eq!(decoded, data);
    assert_eq!(decoded.fields().len(), 3);
    assert_eq!(
        decoded
            .typed_fields()
            .map(|(index, _)| index)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
}

#[test]
fn wholly_opaque_payload_is_not_fieldless() {
    let data = ErrorVariantData::Tuple(vec![ErrorField::Opaque, ErrorField::Opaque]);
    assert_eq!(data.fields().len(), 2);
    assert!(data.typed_fields().next().is_none());
}

fn error_library(data: ErrorVariantData) -> Library {
    serde_json::from_value(json!({
        "prefix": "test",
        "type_registry": {
            "PayloadError": {"kind": {"error": {"c_name": "TestPayloadError"}}, "type_tag": 1},
            "u64": {"kind": {"primitive": {"c_type": "uint64_t"}}},
            "u16": {"kind": {"primitive": {"c_type": "uint16_t"}}},
            "PayloadAlias": {"kind": {"alias": {"alias_of": "PayloadFlags"}}},
            "PayloadFlags": {"kind": {"bitflags": {"alias_of": "PayloadEnum"}}},
            "PayloadEnum": {"kind": {"enum": {"alias_of": "u16"}}},
            "Unrelated": {"kind": {"primitive": {"c_type": "bool"}}}
        },
        "exported_types": [],
        "traits": [],
        "trait_impls": [],
        "errors": [{
            "name": "PayloadError",
            "variants": [{
                "name": "Payload",
                "c_name": "TEST_ERROR_PAYLOAD",
                "code": 1,
                "message": "Payload",
                "data": data
            }]
        }]
    }))
    .unwrap()
}

#[test]
fn pruning_retains_types_referenced_only_by_error_payloads() {
    let mut library = error_library(ErrorVariantData::Tuple(vec![
        ErrorField::Opaque,
        typed_field("u64"),
        ErrorField::Opaque,
        typed_field("PayloadAlias"),
    ]));
    let names = [
        "PayloadError",
        "u64",
        "PayloadAlias",
        "PayloadFlags",
        "PayloadEnum",
        "u16",
    ];
    let referenced = library.referenced_types();
    for name in names {
        assert!(
            referenced.contains(name),
            "missing payload reference {name}"
        );
    }
    library.prune_unreferenced_types();
    for name in names {
        assert!(
            library.type_registry.contains_key(name),
            "pruned payload type {name}"
        );
    }
    assert!(!library.type_registry.contains_key("Unrelated"));
    assert_eq!(library.type_registry.len(), names.len());
}

#[test]
fn fieldless_and_opaque_variants_do_not_keep_unrelated_types() {
    for data in [
        ErrorVariantData::Fieldless,
        ErrorVariantData::Tuple(vec![]),
        ErrorVariantData::Tuple(vec![ErrorField::Opaque]),
    ] {
        let mut library = error_library(data);
        library.prune_unreferenced_types();
        assert_eq!(library.type_registry.len(), 1);
        assert!(library.type_registry.contains_key("PayloadError"));
    }
}
