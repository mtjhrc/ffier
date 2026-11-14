use ffier_schema::{Cfg, Library, Method, TraitImpl};

use crate::cfg;

use super::definition::emit_default_dispatch_body;
use super::implementation::forwarder_cfg;

#[test]
fn default_dispatch_preserves_trait_names_ending_in_vtable() {
    let mut library = Library::from_json(
        r#"{
                "prefix":"test", "type_registry":{}, "exported_types":[],
                "errors":[], "trait_impls":[],
                "traits":[{
                    "name":"ScreenVtable", "destroy_ffi_name":"test_screen_vtable_destroy",
                    "type_tag_constant":"TEST_SCREEN_VTABLE_TYPE_TAG",
                    "vtable_struct_c_name":"TestScreenVtableVtable",
                    "wrapper_c_name":"TestVtableScreenVtable",
                    "vtable_struct_name":"ScreenVtableVtable", "wrapper_name":"VtableScreenVtable",
                    "methods":[{
                        "name":"value", "receiver":"ref", "params":[],
                        "ret":{"return_kind":"void"}, "ffi_name":"test_screen_vtable_value",
                        "trait_definition":{"index":0,"has_default":true}
                    }], "own_method_count":1, "num_slots":1
                }]
            }"#,
    )
    .unwrap();
    for (name, vtable) in [
        ("ScreenVtable", "ScreenVtableVtable"),
        ("ScreenVtableVtable", "CustomCallbackLayout"),
    ] {
        library.traits[0].name = name.into();
        library.traits[0].vtable_struct_name = vtable.into();
        let method = &library.traits[0].methods[0];
        let mut output = String::new();
        emit_default_dispatch_body(&mut output, method, &library, &library.traits[0], 1, 0);
        assert!(output.contains(&format!("__ffier_vtable_{name}::<Self>()")));
        assert!(output.contains(&format!("&'static {vtable}")));
        assert!(!output.contains("__ffier_vtable_Screen::<Self>()"));
    }
}

/// Build a trait default method and an impl method sharing its name.
fn default_and_override(
    default_cfg: Option<Cfg>,
    override_cfg: Option<Cfg>,
) -> (Method, TraitImpl) {
    let method = |cfg: Option<Cfg>, has_default: bool| Method {
        name: "go".into(),
        doc: Vec::new(),
        receiver: ffier_schema::Receiver::Ref,
        receiver_lifetime: None,
        method_lifetimes: Vec::new(),
        params: Vec::new(),
        ret: ffier_schema::Return::Void,
        ffi_name: "probe_go".into(),
        trait_definition: Some(ffier_schema::TraitMethodDefinition {
            index: 0,
            has_default,
        }),
        cfg,
    };
    let ti = TraitImpl {
        trait_name: "T".into(),
        struct_name: "Widget".into(),
        lifetimes: Vec::new(),
        trait_lifetime_args: Vec::new(),
        struct_lifetime_args: Vec::new(),
        methods: vec![method(override_cfg, false)],
        cfg: None,
    };
    (method(default_cfg, true), ti)
}

#[test]
fn forwarder_is_skipped_when_impl_supplies_an_unconditional_body() {
    let (dm, ti) = default_and_override(Some(Cfg::Unix), None);
    assert_eq!(forwarder_cfg(&dm, &ti), None);
}

#[test]
fn forwarder_is_skipped_when_the_same_variant_is_overridden() {
    // A user-level `#[cfg(unix)]` reaches the generator as `unix` for a trait
    // item but as `all(unix)` for an impl item.
    let (dm, ti) = default_and_override(Some(Cfg::Unix), Some(Cfg::All(vec![Cfg::Unix])));
    assert_eq!(forwarder_cfg(&dm, &ti), None);
}

#[test]
fn forwarder_is_emitted_for_the_variant_the_impl_does_not_override() {
    let (dm, ti) = default_and_override(
        Some(Cfg::All(vec![Cfg::Not(Box::new(Cfg::Windows))])),
        Some(Cfg::Windows),
    );
    let cfg = forwarder_cfg(&dm, &ti).flatten().expect("forwarder");
    let rendered = cfg.to_string();
    // Keeps the default's own guard and excludes the overridden variant.
    assert!(rendered.contains("not(windows)"), "{rendered}");
    assert!(
        rendered.contains("windows"),
        "excludes override: {rendered}"
    );
}

#[test]
fn forwarder_excludes_the_relaxed_feature_guard_not_the_raw_predicate() {
    // Overrides on `feature = "x"` are emitted relaxed for dynamic lookup, so
    // the forwarder must negate that guard. Negating the raw predicate would
    // leave both true under `ffier-dynlookup`.
    let feature = Cfg::Feature("x".into());
    let (dm, ti) = default_and_override(None, Some(feature.clone()));
    let cfg = forwarder_cfg(&dm, &ti).flatten().expect("forwarder");
    let override_guard = cfg::client_cfg(&feature);
    // The relaxed guard mentions `ffier-strong`; the raw predicate does not.
    assert_ne!(override_guard, feature.to_string());
    assert!(
        cfg.to_string().contains("ffier-strong"),
        "must negate the emitted guard: {}",
        cfg
    );
    assert!(
        !cfg.to_string().contains(&format!("not({feature})")),
        "must not negate the raw predicate: {}",
        cfg
    );
}
