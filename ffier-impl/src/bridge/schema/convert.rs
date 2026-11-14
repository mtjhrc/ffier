//! Convert each metadata item into its language-independent schema shape.

use std::collections::{BTreeMap, HashSet};

use proc_macro2::TokenStream as TokenStream2;

use crate::meta::{
    MetaBitflags, MetaEnum, MetaError, MetaErrorField, MetaErrorVariantData, MetaExportable,
    MetaFreeFunction, MetaImplementable, MetaMethod, MetaMethodContext, MetaParamKind,
    MetaReceiver, MetaReturn, MetaTraitImpl, MetaValueStruct, camel_to_snake, camel_to_upper_snake,
    extract_result_ok_type, is_result_ok_handle, option_value_struct_inner,
};

use super::super::returns::is_borrowed_handle;
use super::types::CTypeResolver;

/// Convert parsed Rust metadata into a language-independent predicate tree.
fn cfg_to_schema(cfg: Option<&TokenStream2>) -> Option<ffier_schema::Cfg> {
    cfg.map(|tokens| crate::cfg::from_tokens(tokens).expect("valid cfg predicate"))
}

pub(super) fn convert_value_struct(
    meta: &MetaValueStruct,
    r: &CTypeResolver,
) -> syn::Result<ffier_schema::ValueStruct> {
    let name = meta.name.to_string();
    Ok(ffier_schema::ValueStruct {
        c_name: r.handle_c_name(&name),
        fields: meta
            .fields
            .iter()
            .map(|field| {
                Ok(ffier_schema::ValueField {
                    name: field.name.to_string(),
                    type_ref: r.type_ref_from_tokens(&field.rust_type)?,
                })
            })
            .collect::<syn::Result<_>>()?,
        cfg: cfg_to_schema(meta.cfg_predicate.as_ref()),
        name,
    })
}

pub(super) fn convert_enum(meta: &MetaEnum, r: &CTypeResolver) -> ffier_schema::EnumType {
    let name = meta.name.to_string();
    let stripped = name.as_str();
    let name_upper = camel_to_upper_snake(stripped);
    ffier_schema::EnumType {
        name: name.clone(),
        variants: meta
            .variants
            .iter()
            .map(|v| {
                let variant_upper = camel_to_upper_snake(&v.name.to_string());
                ffier_schema::EnumVariant {
                    name: v.name.to_string(),
                    c_name: format!("{}{}_{}", r.upper_pfx, name_upper, variant_upper),
                    value: v.value,
                }
            })
            .collect(),
        cfg: cfg_to_schema(meta.cfg_predicate.as_ref()),
    }
}

pub(super) fn convert_bitflags(meta: &MetaBitflags, r: &CTypeResolver) -> ffier_schema::EnumType {
    let name = meta.name.to_string();
    let name_upper = camel_to_upper_snake(&name);
    ffier_schema::EnumType {
        name: name.clone(),
        variants: meta
            .variants
            .iter()
            .map(|v| {
                // Bitflags constant names are already UPPER_SNAKE_CASE
                // (e.g. READ, WRITE), so use them verbatim — don't run
                // camel_to_upper_snake which would split "READ" into "R_E_A_D".
                let variant_name = v.name.to_string();
                ffier_schema::EnumVariant {
                    name: variant_name.clone(),
                    c_name: format!("{}{}_{}", r.upper_pfx, name_upper, variant_name),
                    value: v.value,
                }
            })
            .collect(),
        cfg: cfg_to_schema(meta.cfg_predicate.as_ref()),
    }
}

pub(super) fn convert_free_fn(
    meta: &MetaFreeFunction,
    m: &MetaMethod,
    r: &CTypeResolver,
    handle_types: &HashSet<String>,
    type_registry: &BTreeMap<String, ffier_schema::TypeEntry>,
    value_types: &HashSet<String>,
) -> syn::Result<ffier_schema::FreeFunction> {
    // Use type-level cfg_predicate if set, otherwise fall back to method-level cfg.
    let cfg = meta.cfg_predicate.as_ref().or(m.cfg.as_ref());
    Ok(ffier_schema::FreeFunction {
        name: meta.name.to_string(),
        ffi_name: r.ffi_fn_name(&meta.ffi_name),
        doc: m.doc.clone(),
        lifetimes: m.method_lifetimes.iter().map(ToString::to_string).collect(),
        params: m
            .params
            .iter()
            .map(|p| convert_param(p, r, type_registry))
            .collect::<syn::Result<_>>()?,
        ret: convert_return(&m.ret, &m.rust_ret, r, false, handle_types, value_types)?,
        cfg: cfg_to_schema(cfg),
    })
}

pub(super) fn convert_exportable(
    meta: &MetaExportable,
    r: &CTypeResolver,
    handle_types: &HashSet<String>,
    type_registry: &BTreeMap<String, ffier_schema::TypeEntry>,
    value_types: &HashSet<String>,
) -> syn::Result<ffier_schema::ExportedType> {
    let name = meta.struct_name.to_string();
    let name_snake = camel_to_snake(&name);
    let is_builder_type = meta
        .methods
        .iter()
        .any(|m| m.is_builder() && m.receiver == MetaReceiver::Value);
    Ok(ffier_schema::ExportedType {
        name,
        destroy_ffi_name: r.ffi_fn_name(&format!("{name_snake}_destroy")),
        is_builder_type,
        thread_safety: meta.thread_safety,
        methods: meta
            .methods
            .iter()
            .map(|m| convert_method(m, r, None, handle_types, type_registry, value_types))
            .collect::<syn::Result<_>>()?,
        cfg: cfg_to_schema(meta.cfg_predicate.as_ref()),
    })
}

pub(super) fn convert_error(
    meta: &MetaError,
    r: &CTypeResolver,
) -> syn::Result<ffier_schema::ErrorType> {
    Ok(ffier_schema::ErrorType {
        name: meta.name.to_string(),
        variants: meta
            .variants
            .iter()
            .map(|v| {
                let data = match &v.data {
                    MetaErrorVariantData::Fieldless => ffier_schema::ErrorVariantData::Fieldless,
                    MetaErrorVariantData::Tuple(fields) => ffier_schema::ErrorVariantData::Tuple(
                        fields
                            .iter()
                            .map(|field| {
                                Ok(match field {
                                    MetaErrorField::Typed(tokens) => {
                                        ffier_schema::ErrorField::Typed(
                                            r.type_ref_from_tokens(tokens)?,
                                        )
                                    }
                                    MetaErrorField::Opaque => ffier_schema::ErrorField::Opaque,
                                })
                            })
                            .collect::<syn::Result<_>>()?,
                    ),
                };
                Ok(ffier_schema::ErrorVariant {
                    name: v.name.to_string(),
                    c_name: r.error_const_name(&meta.name.to_string(), &v.name.to_string()),
                    code: v.code,
                    message: v.message.clone(),
                    data,
                })
            })
            .collect::<syn::Result<_>>()?,
    })
}

pub(super) fn convert_implementable(
    meta: &MetaImplementable,
    r: &CTypeResolver,
    handle_types: &HashSet<String>,
    type_registry: &BTreeMap<String, ffier_schema::TypeEntry>,
    value_types: &HashSet<String>,
) -> syn::Result<ffier_schema::ImplementableTrait> {
    let name = meta.trait_name.to_string();
    let name_snake = camel_to_snake(&name);
    let ffi_prefix = format!("{name_snake}_");
    let name_upper_snake = camel_to_snake(&name).to_ascii_uppercase();
    Ok(ffier_schema::ImplementableTrait {
        name: name.clone(),
        destroy_ffi_name: r.ffi_fn_name(&format!("{name_snake}_destroy")),
        type_tag_constant: format!("{}{name_upper_snake}_TYPE_TAG", r.upper_pfx),
        vtable_struct_c_name: format!("{}{}Vtable", r.type_pfx, name),
        wrapper_c_name: format!("{}Vtable{}", r.type_pfx, name),
        vtable_struct_name: format!("{name}Vtable"),
        wrapper_name: format!("Vtable{name}"),
        thread_safety: meta.thread_safety,
        methods: meta
            .methods
            .iter()
            .map(|m| {
                convert_method(
                    m,
                    r,
                    Some(&ffi_prefix),
                    handle_types,
                    type_registry,
                    value_types,
                )
            })
            .collect::<syn::Result<_>>()?,
        own_method_count: meta.own_method_count,
        num_slots: meta.num_slots,
        no_vtable: meta.no_vtable,
        cfg: cfg_to_schema(meta.cfg_predicate.as_ref()),
    })
}

pub(super) fn convert_trait_impl(
    meta: &MetaTraitImpl,
    r: &CTypeResolver,
    handle_types: &HashSet<String>,
    type_registry: &BTreeMap<String, ffier_schema::TypeEntry>,
    value_types: &HashSet<String>,
) -> syn::Result<ffier_schema::TraitImpl> {
    Ok(ffier_schema::TraitImpl {
        trait_name: meta.trait_name.to_string(),
        struct_name: meta.struct_name.to_string(),
        lifetimes: meta.lifetimes.iter().map(|lt| lt.to_string()).collect(),
        trait_lifetime_args: meta.trait_lifetime_args.clone(),
        struct_lifetime_args: meta.struct_lifetime_args.clone(),
        methods: meta
            .methods
            .iter()
            .map(|m| convert_method(m, r, None, handle_types, type_registry, value_types))
            .collect::<syn::Result<_>>()?,
        cfg: cfg_to_schema(meta.cfg_predicate.as_ref()),
    })
}

/// Convert a method to its schema representation.
/// `parent_ffi_prefix` is the `"{type_snake}_"` prefix for the parent type/trait
/// (e.g. `"fruit_"` for an exported trait).
/// Only needed for trait definition methods; concrete methods already carry their own ffi_name.
fn convert_method(
    meta: &MetaMethod,
    r: &CTypeResolver,
    parent_ffi_prefix: Option<&str>,
    handle_types: &HashSet<String>,
    type_registry: &BTreeMap<String, ffier_schema::TypeEntry>,
    value_types: &HashSet<String>,
) -> syn::Result<ffier_schema::Method> {
    let (ffi_name, trait_definition) = match &meta.context {
        MetaMethodContext::Exportable { ffi_name, .. } => (r.ffi_fn_name(ffi_name), None),
        MetaMethodContext::Trait {
            has_default, index, ..
        } => {
            let prefix = parent_ffi_prefix.expect("trait method requires parent_ffi_prefix");
            let ffi_name = r.ffi_fn_name(&format!("{prefix}{}", meta.name));
            let td = ffier_schema::TraitMethodDefinition {
                index: *index,
                has_default: *has_default,
            };
            (ffi_name, Some(td))
        }
    };

    let ret = convert_return(
        &meta.ret,
        &meta.rust_ret,
        r,
        meta.is_builder(),
        handle_types,
        value_types,
    )?;

    Ok(ffier_schema::Method {
        name: meta.name.to_string(),
        doc: meta.doc().to_vec(),
        receiver: convert_receiver(meta.receiver),
        receiver_lifetime: meta.receiver_lifetime.as_ref().map(ToString::to_string),
        method_lifetimes: meta
            .method_lifetimes
            .iter()
            .map(|lt| lt.to_string())
            .collect(),
        params: meta
            .params
            .iter()
            .map(|p| convert_param(p, r, type_registry))
            .collect::<syn::Result<_>>()?,
        ret,
        ffi_name,
        trait_definition,
        cfg: cfg_to_schema(meta.cfg.as_ref()),
    })
}

fn convert_receiver(recv: MetaReceiver) -> ffier_schema::Receiver {
    match recv {
        MetaReceiver::None => ffier_schema::Receiver::None,
        MetaReceiver::Ref => ffier_schema::Receiver::Ref,
        MetaReceiver::Mut => ffier_schema::Receiver::Mut,
        MetaReceiver::Value => ffier_schema::Receiver::Value,
    }
}

fn convert_param(
    p: &crate::meta::MetaParam,
    r: &CTypeResolver,
    type_registry: &BTreeMap<String, ffier_schema::TypeEntry>,
) -> syn::Result<ffier_schema::Param> {
    let param_type = match &p.kind {
        MetaParamKind::Regular(tp) => {
            ffier_schema::ParamType::Regular(r.type_ref_from_tokens(&tp.rust_type)?)
        }
        MetaParamKind::StrSlice => {
            // &[&str] → two C params: pointer to FfierBytes array + length.
            // The element type is &str (a reference to str).
            let str_c = format!("{}Str", r.prim_type_pfx);
            ffier_schema::ParamType::Slice {
                element: ffier_schema::TypeRef {
                    type_name: "str".to_string(),
                    ref_kind: ffier_schema::RefKind::Shared,
                    ref_lifetime: None,
                    type_args: vec![],
                    optional: false,
                    owned: false,
                },
                c_params: vec![
                    ffier_schema::CParam {
                        name: p.name.to_string(),
                        c_type: format!("const {str_c}*"),
                    },
                    ffier_schema::CParam {
                        name: format!("{}_len", p.name),
                        c_type: "size_t".to_string(),
                    },
                ],
            }
        }
        MetaParamKind::HandleSlice(tp) => {
            // &[&T] → two C params: pointer to handle array + length.
            let elem_ref = r.type_ref_from_tokens(&tp.rust_type)?;
            let handle_c = r.handle_c_name(&elem_ref.type_name);
            ffier_schema::ParamType::Slice {
                element: ffier_schema::TypeRef {
                    type_name: elem_ref.type_name,
                    ref_kind: ffier_schema::RefKind::Shared,
                    ref_lifetime: None,
                    type_args: vec![],
                    optional: false,
                    owned: false,
                },
                c_params: vec![
                    ffier_schema::CParam {
                        name: p.name.to_string(),
                        c_type: format!("const {handle_c}*"),
                    },
                    ffier_schema::CParam {
                        name: format!("{}_len", p.name),
                        c_type: "size_t".to_string(),
                    },
                ],
            }
        }
        MetaParamKind::PrimitiveSlice(tp) => {
            // &[T] where T is a primitive → two C params: pointer to T array + length.
            let elem_ref = r.type_ref_from_tokens(&tp.rust_type)?;
            let entry = type_registry.get(&elem_ref.type_name).unwrap_or_else(|| {
                panic!(
                    "primitive type `{}` not found in type_registry",
                    elem_ref.type_name
                )
            });
            let elem_c = match &entry.kind {
                ffier_schema::TypeKind::Primitive { c_type } => c_type.as_str(),
                _ => panic!(
                    "expected primitive type for `{}`, got {:?}",
                    elem_ref.type_name, entry.kind
                ),
            };
            ffier_schema::ParamType::Slice {
                element: ffier_schema::TypeRef {
                    type_name: elem_ref.type_name,
                    ref_kind: ffier_schema::RefKind::None,
                    ref_lifetime: None,
                    type_args: vec![],
                    optional: false,
                    owned: false,
                },
                c_params: vec![
                    ffier_schema::CParam {
                        name: p.name.to_string(),
                        c_type: format!("const {elem_c}*"),
                    },
                    ffier_schema::CParam {
                        name: format!("{}_len", p.name),
                        c_type: "size_t".to_string(),
                    },
                ],
            }
        }
        MetaParamKind::ImplTrait {
            trait_name,
            trait_lifetime_args,
            lifetime_bounds,
            ref_kind,
            ref_lifetime,
            ..
        } => ffier_schema::ParamType::ImplTrait {
            trait_name: trait_name.clone(),
            type_args: trait_lifetime_args
                .iter()
                .map(|lt| lt.to_string())
                .collect(),
            lifetime_bounds: lifetime_bounds.iter().map(ToString::to_string).collect(),
            ref_kind: match ref_kind {
                crate::meta::ImplTraitRefKind::Value => ffier_schema::RefKind::None,
                crate::meta::ImplTraitRefKind::Ref => ffier_schema::RefKind::Shared,
                crate::meta::ImplTraitRefKind::Mut => ffier_schema::RefKind::Mut,
            },
            ref_lifetime: ref_lifetime.as_ref().map(ToString::to_string),
        },
    };
    Ok(ffier_schema::Param {
        name: p.name.to_string(),
        param_type,
    })
}

fn builder_self_type_ref() -> ffier_schema::TypeRef {
    ffier_schema::TypeRef {
        type_name: ffier_schema::SELF_TYPE.to_string(),
        ref_kind: ffier_schema::RefKind::None,
        ref_lifetime: None,
        type_args: vec![],
        optional: false,
        owned: false,
    }
}

fn convert_return(
    ret: &MetaReturn,
    rust_ret: &TokenStream2,
    r: &CTypeResolver,
    is_builder: bool,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
) -> syn::Result<ffier_schema::Return> {
    Ok(match ret {
        MetaReturn::Void if is_builder => {
            // `-> Self`: encode as Value(Self).
            ffier_schema::Return::Value(builder_self_type_ref())
        }
        MetaReturn::Void => ffier_schema::Return::Void,
        MetaReturn::Value(tp) => {
            if let Some(inner) = option_value_struct_inner(&tp.rust_type, value_types) {
                return Ok(ffier_schema::Return::OptionalValue(
                    r.type_ref_from_tokens(&inner)?,
                ));
            }
            ffier_schema::Return::Value(r.type_ref_from_tokens(&tp.rust_type)?)
        }
        MetaReturn::HandleSlice { types, .. } => {
            // &[&T] or &[T] → returns FfierObjectArray with element type info.
            ffier_schema::Return::ObjectArray {
                element: r.type_ref_from_tokens(&types.rust_type)?,
            }
        }
        MetaReturn::Result { ok, err_ident } if is_builder => {
            // Builder `-> Result<Self, E>`: ok was suppressed to None by
            // annotations; restore it as Self.
            let ok_ref = match ok {
                None => Some(builder_self_type_ref()),
                Some(tp) => Some(r.type_ref_from_tokens(&tp.rust_type)?),
            };
            let ok_is_handle = ok.is_some() && is_result_ok_handle(rust_ret, handle_types);
            ffier_schema::Return::Result {
                ok: ok_ref,
                err_type: err_ident.clone(),
                c_convention: if ok_is_handle {
                    ffier_schema::CResultConvention::HandleOrNull
                } else {
                    ffier_schema::CResultConvention::OutParam
                },
            }
        }
        MetaReturn::Result { ok, err_ident } => {
            // `Result<Option<ValueStruct>, E>` keeps the value type and marks it
            // optional; the presence out-param is implied by the convention.
            if let Some(tp) = ok
                && let Some(inner) = option_value_struct_inner(&tp.rust_type, value_types)
            {
                let mut ok_ref = r.type_ref_from_tokens(&inner)?;
                ok_ref.optional = true;
                return Ok(ffier_schema::Return::Result {
                    ok: Some(ok_ref),
                    err_type: err_ident.clone(),
                    c_convention: ffier_schema::CResultConvention::OptionalValueOutParam,
                });
            }
            let ok_ref = ok
                .as_ref()
                .map(|tp| r.type_ref_from_tokens(&tp.rust_type))
                .transpose()?;
            let ok_is_foreign = ok.as_ref().is_some_and(|tp| tp.foreign_crate.is_some());
            let ok_is_handle =
                ok_is_foreign || (ok.is_some() && is_result_ok_handle(rust_ret, handle_types));
            let ok_is_borrowed_handle = !ok_is_foreign && ok.is_some() && {
                let ok_tokens = extract_result_ok_type(rust_ret);
                syn::parse2::<syn::Type>(ok_tokens)
                    .ok()
                    .is_some_and(|ty| is_borrowed_handle(&ty, handle_types))
            };
            ffier_schema::Return::Result {
                ok: ok_ref,
                err_type: err_ident.clone(),
                c_convention: if ok_is_handle || ok_is_borrowed_handle {
                    ffier_schema::CResultConvention::HandleOrNull
                } else {
                    ffier_schema::CResultConvention::OutParam
                },
            }
        }
    })
}
