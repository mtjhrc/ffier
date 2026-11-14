//! Schema conversion and registry invariants.

use ffier_schema::{Blessing, LifetimeParam, LifetimeVariance, TypeEntry, TypeKind};
use quote::quote;

use crate::meta::{MetaError, MetaErrorVariantData};

use super::convert::convert_error;
use super::types::CTypeResolver;
use super::{RegistrySource, SchemaRegistry};

fn lifetime(name: &str, variance: Option<LifetimeVariance>) -> LifetimeParam {
    LifetimeParam {
        name: name.to_string(),
        variance,
    }
}

#[test]
fn error_metadata_preserves_shape_and_opaque_field_positions() {
    use ffier_schema::{ErrorField, ErrorVariantData};

    let mut meta: MetaError = syn::parse2(quote! {
        @exported_error,
        name = Example,
        path = (crate::Example),
        prefix = "test",
        type_tag = 1,
        variants = [
            { name = Plain, code = 1, message = "plain", data = fieldless, },
            { name = Empty, code = 2, message = "empty", data = tuple [], },
            { name = UnitPayload, code = 3, message = "unit payload", data = tuple [typed(())], },
            { name = Mixed, code = 4, message = "mixed", data = tuple [typed(i32), opaque, typed(u64)], },
            { name = Hidden, code = 5, message = "hidden", data = tuple [opaque, opaque], },
        ],
    }).unwrap();
    let error = convert_error(&meta, &CTypeResolver::new("test", None)).unwrap();
    assert_eq!(error.variants[0].data, ErrorVariantData::Fieldless);
    assert_eq!(error.variants[1].data, ErrorVariantData::Tuple(vec![]));
    assert_eq!(error.variants[2].data.fields().len(), 1);
    assert!(matches!(
        error.variants[2].data.fields(),
        [ErrorField::Typed(_)]
    ));
    let mixed = &error.variants[3].data;
    assert!(matches!(
        mixed.fields(),
        [
            ErrorField::Typed(_),
            ErrorField::Opaque,
            ErrorField::Typed(_)
        ]
    ));
    assert_eq!(
        mixed
            .typed_fields()
            .map(|(index, _)| index)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert_eq!(
        error.variants[4].data,
        ErrorVariantData::Tuple(vec![ErrorField::Opaque, ErrorField::Opaque])
    );
    let expected = serde_json::to_value(&error.variants[4]).unwrap();
    for variant in [
        syn::parse_quote! {
            #[ffier(code = 5, message = "hidden", opaque)]
            Hidden(i32, anyhow::Error)
        },
        syn::parse_quote! {
            #[ffier(code = 5, message = "hidden")]
            Hidden(#[ffier(opaque)] i32, #[ffier(opaque)] anyhow::Error)
        },
    ] {
        let variant: syn::Variant = variant;
        let attrs = crate::error::parse_ffier_variant_attrs(&variant.attrs).unwrap();
        let syn::Fields::Unnamed(fields) = &variant.fields else {
            panic!("expected tuple variant");
        };
        let fields = fields
            .unnamed
            .iter()
            .map(|field| crate::error::parse_ffier_error_field(field, attrs.opaque).unwrap())
            .collect();
        meta.variants = vec![crate::meta::MetaErrorVariant {
            name: variant.ident,
            code: attrs.code,
            message: attrs.message.unwrap(),
            data: MetaErrorVariantData::Tuple(fields),
        }];
        let converted = convert_error(&meta, &CTypeResolver::new("test", None)).unwrap();
        assert_eq!(
            serde_json::to_value(&converted.variants[0]).unwrap(),
            expected
        );
    }
}

#[test]
fn qualified_paths_preserve_schema_type_information() {
    let resolver = CTypeResolver::new("test", None);
    for (source, canonical) in [
        ("std::option::Option<&str>", "Option<&str>"),
        ("alloc::boxed::Box<str>", "Box<str>"),
        (
            "::core::result::Result<::std::option::Option<::std::boxed::Box<str>>, Error>",
            "Option<Box<str>>",
        ),
    ] {
        let source: syn::Type = syn::parse_str(source).unwrap();
        let source = crate::types::extract_result_types(&source).map_or(source, |(ok, _)| ok);
        let canonical: syn::Type = syn::parse_str(canonical).unwrap();
        assert_eq!(
            resolver.type_ref_from_syn(&source).unwrap(),
            resolver.type_ref_from_syn(&canonical).unwrap(),
            "source: {source:?}",
        );
    }
}

#[test]
fn registry_rejects_duplicate_declarations_without_overwriting() {
    let mut registry = SchemaRegistry::default();
    let entry = TypeEntry {
        kind: TypeKind::Error {
            c_name: "TestError".to_string(),
        },
        type_tag: Some(1),
        bless: None,
        lifetime_params: vec![],
    };
    registry
        .register(
            "Error",
            entry.clone(),
            RegistrySource::Declaration,
            proc_macro2::Span::call_site(),
        )
        .unwrap();
    // Even an identical declaration is a duplicate, not a reference.
    assert!(
        registry
            .register(
                "Error",
                entry.clone(),
                RegistrySource::Declaration,
                proc_macro2::Span::call_site()
            )
            .is_err()
    );
    let conflicting = TypeEntry {
        kind: TypeKind::Trait {
            c_name: "TestError".to_string(),
        },
        type_tag: Some(2),
        bless: Some(Blessing::ErrorTrait),
        lifetime_params: vec![],
    };
    let error = registry
        .register(
            "Error",
            conflicting,
            RegistrySource::Declaration,
            proc_macro2::Span::call_site(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("FFI type name `Error` is already registered")
    );
    assert_eq!(registry.into_entries()["Error"], entry);
}

#[test]
fn registry_compares_foreign_metadata_without_lifetime_names() {
    let mut registry = SchemaRegistry::default();
    let entry = TypeEntry {
        kind: TypeKind::ForeignHandle {
            foreign_crate: "foreign".to_string(),
            c_name: "ForeignView".to_string(),
        },
        type_tag: None,
        bless: None,
        lifetime_params: vec![lifetime("a", None)],
    };
    registry
        .register(
            "View",
            entry.clone(),
            RegistrySource::ForeignReference,
            proc_macro2::Span::call_site(),
        )
        .unwrap();
    let renamed = TypeEntry {
        lifetime_params: vec![lifetime("b", None)],
        ..entry.clone()
    };
    registry
        .register(
            "View",
            renamed.clone(),
            RegistrySource::ForeignReference,
            proc_macro2::Span::call_site(),
        )
        .unwrap();
    registry
        .register(
            "View",
            TypeEntry {
                lifetime_params: vec![lifetime("static", None)],
                ..entry.clone()
            },
            RegistrySource::ForeignReference,
            proc_macro2::Span::call_site(),
        )
        .unwrap();
    for conflicting in [
        TypeEntry {
            lifetime_params: vec![],
            ..renamed.clone()
        },
        TypeEntry {
            type_tag: Some(1),
            ..renamed.clone()
        },
        TypeEntry {
            bless: Some(Blessing::Object),
            ..renamed.clone()
        },
        TypeEntry {
            kind: TypeKind::ForeignHandle {
                foreign_crate: "another".to_string(),
                c_name: "ForeignView".to_string(),
            },
            ..renamed.clone()
        },
        TypeEntry {
            kind: TypeKind::ForeignHandle {
                foreign_crate: "foreign".to_string(),
                c_name: "AnotherView".to_string(),
            },
            ..renamed.clone()
        },
    ] {
        assert!(
            registry
                .register(
                    "View",
                    conflicting,
                    RegistrySource::ForeignReference,
                    proc_macro2::Span::call_site()
                )
                .is_err()
        );
    }
    // A second declaration still conflicts with an existing reference.
    assert!(
        registry
            .register(
                "View",
                renamed,
                RegistrySource::Declaration,
                proc_macro2::Span::call_site()
            )
            .is_err()
    );
    assert_eq!(registry.into_entries()["View"], entry);
}

#[test]
fn registry_preserves_declared_trait_metadata_across_impl_references() {
    let mut registry = SchemaRegistry::default();
    let entry = TypeEntry {
        kind: TypeKind::Trait {
            c_name: "TestTrait".to_string(),
        },
        type_tag: Some(10),
        bless: Some(Blessing::ErrorTrait),
        lifetime_params: vec![lifetime("a", Some(LifetimeVariance::Invariant))],
    };
    registry
        .register(
            "Trait",
            entry.clone(),
            RegistrySource::Declaration,
            proc_macro2::Span::call_site(),
        )
        .unwrap();
    let reference = TypeEntry {
        type_tag: None,
        bless: None,
        lifetime_params: vec![lifetime("b", Some(LifetimeVariance::Invariant))],
        ..entry.clone()
    };
    registry
        .register(
            "Trait",
            reference.clone(),
            RegistrySource::TraitReference,
            proc_macro2::Span::call_site(),
        )
        .unwrap();
    for conflicting in [
        TypeEntry {
            type_tag: Some(11),
            ..reference.clone()
        },
        TypeEntry {
            bless: Some(Blessing::PushStr),
            ..reference.clone()
        },
        TypeEntry {
            lifetime_params: vec![],
            ..reference.clone()
        },
    ] {
        assert!(
            registry
                .register(
                    "Trait",
                    conflicting,
                    RegistrySource::TraitReference,
                    proc_macro2::Span::call_site(),
                )
                .is_err()
        );
    }
    assert!(
        registry
            .register(
                "Trait",
                reference,
                RegistrySource::Declaration,
                proc_macro2::Span::call_site()
            )
            .is_err()
    );
    assert_eq!(registry.into_entries()["Trait"], entry);
}

#[test]
fn registry_accepts_repeated_impl_only_trait_references() {
    let mut registry = SchemaRegistry::default();
    let entry = TypeEntry {
        kind: TypeKind::Trait {
            c_name: "TestSnapshot".to_string(),
        },
        type_tag: None,
        bless: None,
        lifetime_params: vec![lifetime("a", Some(LifetimeVariance::Invariant))],
    };
    for lifetime_name in ["a", "b"] {
        registry
            .register(
                "Snapshot",
                TypeEntry {
                    lifetime_params: vec![lifetime(
                        lifetime_name,
                        Some(LifetimeVariance::Invariant),
                    )],
                    ..entry.clone()
                },
                RegistrySource::TraitReference,
                proc_macro2::Span::call_site(),
            )
            .unwrap();
    }
    assert_eq!(registry.into_entries()["Snapshot"], entry);
}
