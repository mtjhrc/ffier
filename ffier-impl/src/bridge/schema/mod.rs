//! Build the language-independent schema and write its JSON representation.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashSet};

use ffier_schema::{LifetimeParam, LifetimeVariance};
use proc_macro2::TokenStream as TokenStream2;
use syn::spanned::Spanned;

use crate::meta::{
    MetaBitflags, MetaEnum, MetaError, MetaExportable, MetaFreeFunction, MetaImplementable,
    MetaParamKind, MetaReturn, MetaTraitImpl, MetaTypePair, MetaValueStruct,
};

mod convert;
mod types;

use convert::{
    convert_bitflags, convert_enum, convert_error, convert_exportable, convert_free_fn,
    convert_implementable, convert_trait_impl, convert_value_struct,
};
use types::CTypeResolver;

/// Convert parsed metadata to `ffier_schema::Library` and write to the
/// user-specified `schema_output` path. Relative paths are resolved from
/// `CARGO_MANIFEST_DIR`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_json(
    prefix: &str,
    primitives_prefix: Option<&str>,
    schema_output: &str,
    errors: &[TokenStream2],
    exportables: &[TokenStream2],
    implementables: &[TokenStream2],
    trait_impls: &[TokenStream2],
    enum_constants: &[TokenStream2],
    bitflags_constants: &[TokenStream2],
    value_structs: &[TokenStream2],
    free_fns: &[TokenStream2],
) -> syn::Result<()> {
    let library = build_schema(
        prefix,
        primitives_prefix,
        errors,
        exportables,
        implementables,
        trait_impls,
        enum_constants,
        bitflags_constants,
        value_structs,
        free_fns,
    )?;
    // Standard Option/Box/Result syntax has already been converted. Any remaining
    // use of these names would collide with that syntax in generated bindings.
    let payload_types = library
        .errors
        .iter()
        .flat_map(|error| &error.variants)
        .flat_map(|variant| variant.data.typed_fields())
        .map(|(_, type_ref)| type_ref.type_name.as_str());
    for name in library
        .type_registry
        .keys()
        .map(String::as_str)
        .chain(library.referenced_types())
        .chain(payload_types)
    {
        if matches!(name, "Option" | "Box" | "Result") {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("FFI type name `{name}` is reserved; rename the exported type"),
            ));
        }
    }

    let path = std::path::PathBuf::from(schema_output);
    let path = if path.is_absolute() {
        path
    } else {
        let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
            Ok(d) => std::path::PathBuf::from(d),
            Err(_) => return Ok(()),
        };
        manifest_dir.join(path)
    };

    let json = library.to_json();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    // Only write if contents changed to avoid bumping the timestamp
    // (which would trigger unnecessary downstream rebuilds).
    let needs_write = match std::fs::read(&path) {
        Ok(existing) => existing != json.as_bytes(),
        Err(_) => true,
    };
    if needs_write {
        std::fs::write(&path, json).unwrap_or_else(|e| {
            panic!("failed to write {}: {e}", path.display());
        });
    }
    Ok(())
}

/// Construction-time provenance; not part of the serialized schema.
enum RegistrySource {
    /// A declaration owns its registry name; another declaration is an error.
    Declaration,
    /// An impl reuses the trait entry without declaring another trait.
    TraitReference,
    /// Foreign identity comes from the registered name, crate, and C name.
    ForeignReference,
}

struct RegisteredType {
    entry: ffier_schema::TypeEntry,
    source: RegistrySource,
}

#[derive(Default)]
struct SchemaRegistry {
    types: BTreeMap<String, RegisteredType>,
}

impl SchemaRegistry {
    /// Reject duplicate declarations and merge only matching references.
    fn register(
        &mut self,
        name: &str,
        mut entry: ffier_schema::TypeEntry,
        source: RegistrySource,
        span: proc_macro2::Span,
    ) -> syn::Result<()> {
        match self.types.entry(name.to_string()) {
            Entry::Vacant(slot) => {
                slot.insert(RegisteredType { entry, source });
                Ok(())
            }
            Entry::Occupied(slot) => {
                let existing = slot.get();
                let same_reference = match (&existing.source, &source) {
                    (
                        RegistrySource::Declaration | RegistrySource::TraitReference,
                        RegistrySource::TraitReference,
                    ) => {
                        // An impl references a trait; it does not assign its tag
                        // or blessing. Keep those from the trait declaration.
                        entry.type_tag = entry.type_tag.or(existing.entry.type_tag);
                        entry.bless = entry.bless.or(existing.entry.bless);
                        true
                    }
                    (RegistrySource::ForeignReference, RegistrySource::ForeignReference) => true,
                    _ => false,
                };
                if same_reference && existing.entry.same_registration(&entry) {
                    Ok(())
                } else {
                    Err(syn::Error::new(
                        span,
                        format!(
                            "FFI type name `{name}` is already registered; rename the conflicting type"
                        ),
                    ))
                }
            }
        }
    }

    fn into_entries(self) -> BTreeMap<String, ffier_schema::TypeEntry> {
        self.types
            .into_iter()
            .map(|(name, registered)| (name, registered.entry))
            .collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn build_schema(
    prefix: &str,
    primitives_prefix: Option<&str>,
    errors: &[TokenStream2],
    exportables: &[TokenStream2],
    implementables: &[TokenStream2],
    trait_impls: &[TokenStream2],
    enum_constants: &[TokenStream2],
    bitflags_constants: &[TokenStream2],
    value_structs: &[TokenStream2],
    free_fns: &[TokenStream2],
) -> syn::Result<ffier_schema::Library> {
    let errors_parsed: Vec<_> = errors
        .iter()
        .map(|item| {
            syn::parse2::<MetaError>(item.clone())
                .expect("failed to parse @exported_error metadata")
        })
        .collect();
    let exportables_parsed: Vec<_> = exportables
        .iter()
        .map(|item| syn::parse2::<MetaExportable>(item.clone()))
        .collect::<syn::Result<_>>()?;
    let implementables_parsed: Vec<_> = implementables
        .iter()
        .map(|item| syn::parse2::<MetaImplementable>(item.clone()))
        .collect::<syn::Result<_>>()?;
    let trait_impls_parsed: Vec<_> = trait_impls
        .iter()
        .map(|item| syn::parse2::<MetaTraitImpl>(item.clone()))
        .collect::<syn::Result<_>>()?;
    let enums_parsed: Vec<_> = enum_constants
        .iter()
        .map(|item| syn::parse2::<MetaEnum>(item.clone()))
        .collect::<syn::Result<_>>()?;
    let bitflags_parsed: Vec<_> = bitflags_constants
        .iter()
        .map(|item| syn::parse2::<MetaBitflags>(item.clone()))
        .collect::<syn::Result<_>>()?;
    let free_fns_parsed: Vec<_> = free_fns
        .iter()
        .map(|item| syn::parse2::<MetaFreeFunction>(item.clone()))
        .collect::<syn::Result<_>>()?;
    let value_structs_parsed: Vec<_> = value_structs
        .iter()
        .map(|item| syn::parse2::<MetaValueStruct>(item.clone()))
        .collect::<syn::Result<_>>()?;

    let resolver = CTypeResolver::new(prefix, primitives_prefix);
    let prim_type_pfx = &resolver.prim_type_pfx;

    // Build handle_types set for Result convention detection.
    let handle_types: HashSet<String> = {
        let mut set = HashSet::new();
        for e in &exportables_parsed {
            set.insert(e.struct_name.to_string());
        }
        for i in &implementables_parsed {
            if !i.no_vtable {
                set.insert(format!("Vtable{}", i.trait_name));
            }
        }
        set
    };

    // Build type registry
    let mut registry = SchemaRegistry::default();

    // Primitives
    for (name, c_type) in &[
        ("i8", "int8_t"),
        ("i16", "int16_t"),
        ("i32", "int32_t"),
        ("i64", "int64_t"),
        ("u8", "uint8_t"),
        ("u16", "uint16_t"),
        ("u32", "uint32_t"),
        ("u64", "uint64_t"),
        ("f32", "float"),
        ("f64", "double"),
        ("isize", "ssize_t"),
        ("usize", "size_t"),
        ("bool", "bool"),
        (ffier_schema::C_VOID_PTR, "void*"),
        (ffier_schema::C_VOID_CONST_PTR, "const void*"),
    ] {
        registry.register(
            name,
            ffier_schema::TypeEntry {
                kind: ffier_schema::TypeKind::Primitive {
                    c_type: c_type.to_string(),
                },
                type_tag: None,
                bless: None,
                lifetime_params: vec![],
            },
            RegistrySource::Declaration,
            proc_macro2::Span::call_site(),
        )?;
    }

    // Builtins — use primitives prefix for C names
    registry.register(
        "str",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::String {
                c_name: format!("{prim_type_pfx}Str"),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::Str),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        "[u8]",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Bytes {
                c_name: format!("{prim_type_pfx}Bytes"),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::Bytes),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;

    // Framework types — ABI scaffolding used by generators.
    registry.register(
        "FfierResult",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Primitive {
                c_type: format!("{prim_type_pfx}Result"),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::Result),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        ffier_schema::PATH_TYPE,
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Bytes {
                c_name: format!("{prim_type_pfx}Path"),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::Path),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        "FfierVtableHandle",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Primitive {
                c_type: format!("{prim_type_pfx}VtableHandle"),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::VtableHandle),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        "FfierObjectArray",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Primitive {
                c_type: format!("{prim_type_pfx}ObjectArray"),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::ObjectArray),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        "FfierObject",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Primitive {
                c_type: format!("{prim_type_pfx}Object"),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::Object),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;

    // Std type aliases
    registry.register(
        "RawFd",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Primitive {
                c_type: "int".to_string(),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::RawFd),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        "BorrowedFd",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Alias {
                alias_of: "RawFd".to_string(),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::BorrowedFd),
            lifetime_params: vec![LifetimeParam {
                name: "fd".into(),
                variance: Some(LifetimeVariance::Covariant),
            }],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        "OwnedFd",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Alias {
                alias_of: "RawFd".to_string(),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::OwnedFd),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        "RawHandle",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Primitive {
                c_type: "void *".to_string(),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::RawHandle),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        "BorrowedHandle",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Alias {
                alias_of: "RawHandle".to_string(),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::BorrowedHandle),
            lifetime_params: vec![LifetimeParam {
                name: "handle".into(),
                variance: Some(LifetimeVariance::Covariant),
            }],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;
    registry.register(
        "OwnedHandle",
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Alias {
                alias_of: "RawHandle".to_string(),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::OwnedHandle),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;

    // Enum constants
    for e in &enums_parsed {
        let name = e.name.to_string();
        registry.register(
            &name,
            ffier_schema::TypeEntry {
                kind: ffier_schema::TypeKind::Enum {
                    alias_of: e.repr.clone(),
                },
                type_tag: None,
                bless: None,
                lifetime_params: vec![],
            },
            RegistrySource::Declaration,
            e.name.span(),
        )?;
    }

    // Bitflags constants
    for bf in &bitflags_parsed {
        let name = bf.name.to_string();
        registry.register(
            &name,
            ffier_schema::TypeEntry {
                kind: ffier_schema::TypeKind::Bitflags {
                    alias_of: bf.repr.clone(),
                },
                type_tag: None,
                bless: None,
                lifetime_params: vec![],
            },
            RegistrySource::Declaration,
            bf.name.span(),
        )?;
    }

    // Fixed-layout value structs
    for value in &value_structs_parsed {
        let name = value.name.to_string();
        registry.register(
            &name,
            ffier_schema::TypeEntry {
                kind: ffier_schema::TypeKind::ValueStruct {
                    c_name: resolver.handle_c_name(&name),
                },
                type_tag: None,
                bless: None,
                lifetime_params: vec![],
            },
            RegistrySource::Declaration,
            value.name.span(),
        )?;
    }

    // Handles (exported types)
    for e in &exportables_parsed {
        let name = e.struct_name.to_string();
        registry.register(
            &name,
            ffier_schema::TypeEntry {
                kind: ffier_schema::TypeKind::Handle {
                    c_name: resolver.handle_c_name(&name),
                },
                type_tag: Some(e.type_tag),
                bless: None,
                lifetime_params: e
                    .lifetimes
                    .iter()
                    .map(|(name, variance)| LifetimeParam {
                        name: name.to_string(),
                        variance: Some(*variance),
                    })
                    .collect(),
            },
            RegistrySource::Declaration,
            e.struct_name.span(),
        )?;
    }

    // Errors
    for e in &errors_parsed {
        let name = e.name.to_string();
        registry.register(
            &name,
            ffier_schema::TypeEntry {
                kind: ffier_schema::TypeKind::Error {
                    c_name: resolver.handle_c_name(&name),
                },
                type_tag: Some(e.type_tag),
                bless: None,
                lifetime_params: vec![],
            },
            RegistrySource::Declaration,
            e.name.span(),
        )?;
    }

    // Implementable traits
    for i in &implementables_parsed {
        let name = i.trait_name.to_string();
        registry.register(
            &name,
            ffier_schema::TypeEntry {
                kind: ffier_schema::TypeKind::Trait {
                    c_name: resolver.handle_c_name(&name),
                },
                type_tag: Some(i.type_tag),
                bless: i.bless.as_deref().map(|b| match b {
                    "error_trait" => ffier_schema::Blessing::ErrorTrait,
                    "push_str" => ffier_schema::Blessing::PushStr,
                    _ => panic!("unknown bless value `{b}` — add a Blessing variant for it"),
                }),
                // Rust trait lifetime arguments are invariant.
                lifetime_params: i
                    .trait_lifetimes
                    .iter()
                    .map(|lt| LifetimeParam {
                        name: lt.to_string(),
                        variance: Some(LifetimeVariance::Invariant),
                    })
                    .collect(),
            },
            RegistrySource::Declaration,
            i.trait_name.span(),
        )?;
    }

    // Traits discovered via trait_impls (no @implementable metadata entry).
    // Concrete 'static arguments still occupy a lifetime parameter position.
    // Keep all positions for registration comparison; impl metadata is unchanged.
    for ti in &trait_impls_parsed {
        let name = ti.trait_name.to_string();
        // Rust trait lifetime arguments are invariant.
        let lifetime_params: Vec<LifetimeParam> = ti
            .trait_lifetime_args
            .iter()
            .map(|lt| LifetimeParam {
                name: lt.clone(),
                variance: Some(LifetimeVariance::Invariant),
            })
            .collect();
        registry.register(
            &name,
            ffier_schema::TypeEntry {
                kind: ffier_schema::TypeKind::Trait {
                    c_name: resolver.handle_c_name(&name),
                },
                type_tag: None,
                bless: None,
                lifetime_params,
            },
            RegistrySource::TraitReference,
            ti.trait_name.span(),
        )?;
    }

    // Self sentinel — methods returning Self are void at C ABI level.
    registry.register(
        ffier_schema::SELF_TYPE,
        ffier_schema::TypeEntry {
            kind: ffier_schema::TypeKind::Primitive {
                c_type: "void".to_string(),
            },
            type_tag: None,
            bless: Some(ffier_schema::Blessing::ReplacesSelf),
            lifetime_params: vec![],
        },
        RegistrySource::Declaration,
        proc_macro2::Span::call_site(),
    )?;

    // Foreign handle types — scan all methods/free-functions for foreign type
    // references and register them so generators know they are opaque handles
    // from another ffier library.
    {
        let mut record_foreign = |tp: &MetaTypePair| -> syn::Result<()> {
            if let Some(fc) = &tp.foreign_crate {
                let type_ref = resolver.type_ref_from_tokens(&tp.rust_type)?;
                let type_name = type_ref.type_name;
                let crate_path = fc.to_string().replace(' ', "");
                let entry = ffier_schema::TypeEntry {
                    kind: ffier_schema::TypeKind::ForeignHandle {
                        foreign_crate: crate_path,
                        c_name: tp.foreign_c_name.clone().expect(
                            "foreign type missing c_name (should have been caught earlier)",
                        ),
                    },
                    type_tag: None,
                    bless: None,
                    lifetime_params: type_ref
                        .type_args
                        .into_iter()
                        .map(|name| LifetimeParam {
                            name,
                            variance: None,
                        })
                        .collect(),
                };
                registry.register(
                    &type_name,
                    entry,
                    RegistrySource::ForeignReference,
                    tp.rust_type.span(),
                )?;
            }
            Ok(())
        };
        let all_methods = free_fns_parsed
            .iter()
            .flat_map(|f| f.methods.iter())
            .chain(exportables_parsed.iter().flat_map(|e| e.methods.iter()))
            .chain(implementables_parsed.iter().flat_map(|i| i.methods.iter()))
            .chain(trait_impls_parsed.iter().flat_map(|t| t.methods.iter()));
        for m in all_methods {
            for p in &m.params {
                if let MetaParamKind::Regular(tp) = &p.kind {
                    record_foreign(tp)?;
                }
            }
            match &m.ret {
                MetaReturn::Value(tp) => record_foreign(tp)?,
                MetaReturn::Result { ok: Some(tp), .. } => record_foreign(tp)?,
                _ => {}
            }
        }
    }

    let type_registry = registry.into_entries();

    // Value-struct names, derived from the final registry so both paths agree.
    let value_types: HashSet<String> = value_structs_parsed
        .iter()
        .map(|value| value.name.to_string())
        .collect();

    // Build schema fields while type_registry is still borrowable.
    let exported_types: Vec<_> = exportables_parsed
        .iter()
        .map(|e| convert_exportable(e, &resolver, &handle_types, &type_registry, &value_types))
        .collect::<syn::Result<_>>()?;
    let errors: Vec<_> = errors_parsed
        .iter()
        .map(|e| convert_error(e, &resolver))
        .collect::<syn::Result<_>>()?;
    let enum_constants: Vec<_> = enums_parsed
        .iter()
        .map(|e| convert_enum(e, &resolver))
        .collect();
    let bitflags_constants: Vec<_> = bitflags_parsed
        .iter()
        .map(|bf| convert_bitflags(bf, &resolver))
        .collect();
    let value_structs: Vec<_> = value_structs_parsed
        .iter()
        .map(|v| convert_value_struct(v, &resolver))
        .collect::<syn::Result<_>>()?;
    let free_functions: Vec<_> = free_fns_parsed
        .iter()
        .map(|f| {
            f.methods
                .iter()
                .map(|m| {
                    convert_free_fn(f, m, &resolver, &handle_types, &type_registry, &value_types)
                })
                .collect::<syn::Result<Vec<_>>>()
        })
        .collect::<syn::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let traits: Vec<_> = implementables_parsed
        .iter()
        .map(|i| convert_implementable(i, &resolver, &handle_types, &type_registry, &value_types))
        .collect::<syn::Result<_>>()?;
    let trait_impls: Vec<_> = trait_impls_parsed
        .iter()
        .map(|t| convert_trait_impl(t, &resolver, &handle_types, &type_registry, &value_types))
        .collect::<syn::Result<_>>()?;

    let mut library = ffier_schema::Library {
        prefix: prefix.to_string(),
        primitives_prefix: primitives_prefix.map(|s| s.to_string()),
        type_registry,
        exported_types,
        value_structs,
        errors,
        enum_constants,
        bitflags_constants,
        free_functions,
        traits,
        trait_impls,
    };
    library.prune_unreferenced_types();
    Ok(library)
}

#[cfg(test)]
mod tests;
