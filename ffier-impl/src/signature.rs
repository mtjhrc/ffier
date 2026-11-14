//! Shared signature analysis and method metadata emission.

use quote::{format_ident, quote};
use syn::{FnArg, Pat, ReturnType, Type, ext::IdentExt, visit_mut::VisitMut};

use crate::alias::AliasContext;
use crate::attrs::{extract_doc_comments, parse_ffier_param_attrs};
use crate::cfg::{combine_cfg_predicates, extract_cfg_predicates};
use crate::contracts::thread_safety_bound;
use crate::meta::erase_lifetimes;
use crate::types::{
    extract_result_types, handle_slice_elem, is_handle_slice, is_self_return, is_str_slice,
    is_unit_type, primitive_slice_elem, replace_self_type, type_ident_name,
};

/// A bridge/rust type pair. Used for params and return types.
struct TypePair {
    bridge: proc_macro2::TokenStream,
    rust: proc_macro2::TokenStream,
    /// When set, the type comes from a foreign ffier library and this
    /// is the crate path whose `FfiType`/`FfiHandle` traits should be used.
    foreign_crate: Option<proc_macro2::TokenStream>,
    /// C typedef name for the foreign handle type (e.g. `"FlForeignConfig"`).
    foreign_c_name: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Receiver {
    /// Static method (no receiver).
    None,
    /// `&self`
    Ref,
    /// `&mut self`
    Mut,
    /// `self` (consuming, by value)
    Value,
}

enum ParamKind {
    /// Uniform: bridge_type resolves via `<T as FfiType>::CRepr`.
    Regular,
    /// `&[&str]` — slice of string references, expands to two C params.
    StrSlice,
    /// `&[T]` where T is an exported handle type — slice of struct references,
    /// expands to two C params (pointer to handle array + length).
    HandleSlice,
    /// `&[T]` where T is a primitive type (u32, i64, bool, etc.) —
    /// expands to two C params (pointer to element array + length).
    PrimitiveSlice,
    /// `impl Trait` parameter — generator resolves dispatch types from trait map.
    ImplTrait {
        trait_name: String,
        /// Dispatch mode: "auto", "concrete", or "vtable".
        /// For trait methods this defaults to "auto".
        dispatch: String,
        /// How the param is passed: "value", "ref", or "mut".
        ref_kind: String,
        /// Lifetime arguments on the trait (e.g. `["a"]` for `impl Snapshot<'a>`).
        trait_lifetime_args: Vec<String>,
        lifetime_bounds: Vec<String>,
        ref_lifetime: Option<String>,
    },
}

enum ReturnKind {
    Void,
    Value(TypePair),
    Result {
        ok: Option<TypePair>,
        err_ident: String,
    },
    /// `&[&T]` or `&[T]` where T is an exported handle type — returns a
    /// contiguous array of borrowed handles. `direct` is true for `&[T]`.
    HandleSlice {
        types: TypePair,
        direct: bool,
    },
}

pub(crate) struct ParamInfo {
    name: syn::Ident,
    kind: ParamKind,
    /// Type pair for this param. Present for `Regular` and `ImplTrait`,
    /// `None` for `StrSlice` (which expands to two C params).
    types: Option<TypePair>,
}

pub(crate) struct MethodInfo {
    pub(crate) name: syn::Ident,
    receiver: Receiver,
    /// Explicit reference lifetime on the receiver; elided and `'_` use `None`.
    pub(crate) receiver_lifetime: Option<syn::Ident>,
    pub(crate) params: Vec<ParamInfo>,
    ret: ReturnKind,
    // --- exported-impl-specific (defaults for trait methods) ---
    /// FFI function name suffix (e.g. `"widget_new"`). Empty for trait methods.
    pub(crate) ffi_name: String,
    /// True if this method returns Self (builder pattern).
    is_builder: bool,
    /// Method-level lifetime params.
    method_lifetimes: Vec<syn::Ident>,
    doc_lines: Vec<String>,
    /// Original Rust return type for client codegen.
    rust_ret: Option<proc_macro2::TokenStream>,
    // --- trait-specific (defaults for exported impl methods) ---
    /// Whether this method has a default impl in the trait.
    has_default: bool,
    /// Vtable slot index.
    pub(crate) index: usize,
    /// Raw handle method (receives `*const FfierHandle<Self>` instead of `&self`).
    pub(crate) raw_handle: bool,
    /// `#[cfg(...)]` predicates from the method's attributes. When present,
    /// the bridge function is gated behind this predicate and the schema
    /// records it so generators can emit conditional code.
    pub(crate) cfg_predicates: Vec<syn::Meta>,
}

impl ParamInfo {
    pub(crate) fn is_impl_trait(&self) -> bool {
        matches!(self.kind, ParamKind::ImplTrait { .. })
    }
}

struct ImplTraitInfo {
    trait_name: String,
    trait_lifetime_args: Vec<String>,
    lifetime_bounds: Vec<String>,
}

pub(crate) fn validate_callback_generics(
    generics: &syn::Generics,
    has_callbacks: bool,
) -> syn::Result<()> {
    if has_callbacks
        && (generics.lifetimes().any(|lt| !lt.bounds.is_empty()) || generics.where_clause.is_some())
    {
        return Err(syn::Error::new_spanned(
            generics,
            "ffier callback lifetime bounds on generic parameters and where clauses are unsupported",
        ));
    }
    Ok(())
}

/// Keep trait arguments and value bounds separate; never drop extra bounds.
fn extract_impl_trait_info(ty: &Type) -> syn::Result<Option<ImplTraitInfo>> {
    let Type::ImplTrait(impl_trait) = ty else {
        return Ok(None);
    };
    let mut trait_info = None;
    let mut lifetime_bounds = Vec::new();
    for bound in &impl_trait.bounds {
        match bound {
            syn::TypeParamBound::Lifetime(lt) => lifetime_bounds.push(lt.ident.to_string()),
            syn::TypeParamBound::Trait(tb)
                if trait_info.is_none()
                    && thread_safety_bound(bound).is_none()
                    && tb.lifetimes.is_none()
                    && matches!(tb.modifier, syn::TraitBoundModifier::None) =>
            {
                let seg = tb.path.segments.last().expect("trait path has a segment");
                let mut args = Vec::new();
                match &seg.arguments {
                    syn::PathArguments::None => {}
                    syn::PathArguments::AngleBracketed(arguments) => {
                        for arg in &arguments.args {
                            let syn::GenericArgument::Lifetime(lt) = arg else {
                                return Err(syn::Error::new_spanned(
                                    arg,
                                    "ffier impl Trait supports only lifetime trait arguments",
                                ));
                            };
                            args.push(lt.ident.to_string());
                        }
                    }
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "ffier impl Trait does not support parenthesized trait arguments",
                        ));
                    }
                }
                trait_info = Some((seg.ident.to_string(), args));
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "ffier impl Trait supports one trait bound and lifetime bounds; declare Send/Sync on the callback trait; additional or higher-ranked trait bounds are unsupported",
                ));
            }
        }
    }
    let Some((trait_name, trait_lifetime_args)) = trait_info else {
        return Err(syn::Error::new_spanned(
            ty,
            "ffier impl Trait requires a trait bound",
        ));
    };
    Ok(Some(ImplTraitInfo {
        trait_name,
        trait_lifetime_args,
        lifetime_bounds,
    }))
}

/// Whether the signature's only where predicate is exactly `Self: Sized`.
///
/// Such a clause restricts the method to concrete implementors and therefore
/// never constrains a caller of the generated FFI function, so it carries no
/// information the schema must record.
pub(crate) fn sized_self_where(sig: &syn::Signature) -> bool {
    sig.generics.where_clause.as_ref().is_some_and(|clause| {
        clause.predicates.len() == 1
            && matches!(&clause.predicates[0], syn::WherePredicate::Type(predicate)
                if predicate.lifetimes.is_none()
                    && matches!(&predicate.bounded_ty, Type::Path(ty) if ty.path.is_ident("Self"))
                    && predicate.bounds.len() == 1
                    && matches!(&predicate.bounds[0], syn::TypeParamBound::Trait(bound)
                        if bound.lifetimes.is_none()
                            && matches!(bound.modifier, syn::TraitBoundModifier::None)
                            && bound.path.is_ident("Sized")))
    })
}

/// Parse a single method signature into a unified `MethodInfo`.
///
/// - `attrs`: method-level attributes (for doc comments and `#[ffier(dispatch = ...)]` on params)
/// - `self_ty`: if `Some`, `Self` in param/return types is replaced with this type
///   and builder pattern is detected. Typically `Some` for exported struct impls, `None`
///   for trait definitions and trait impls.
/// - `has_default`: whether this method has a default impl body (trait methods only)
/// - `raw_handle`: whether this is a raw-handle method
#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_method_sig(
    sig: &syn::Signature,
    attrs: &[syn::Attribute],
    ctx: &mut AliasContext,
    self_ty: Option<&Type>,
    has_default: bool,
    raw_handle: bool,
    foreign_return: Option<&syn::Path>,
    foreign_return_c_name: Option<&str>,
) -> syn::Result<Option<MethodInfo>> {
    ctx.current_cfg = combine_cfg_predicates(&extract_cfg_predicates(attrs)?);
    let name = sig.ident.unraw().to_string();
    if name.starts_with('_') {
        return Err(syn::Error::new_spanned(
            &sig.ident,
            format!(
                "exported function or method name `{name}` starts with `_`, which is reserved for generated code"
            ),
        ));
    }

    if let Some(FnArg::Receiver(receiver)) = sig.inputs.first()
        && receiver.colon_token.is_some()
    {
        return Err(syn::Error::new_spanned(
            receiver,
            "ffier typed self parameters are unsupported; use self, &self, or &mut self",
        ));
    }
    let receiver_lifetime = match sig.inputs.first() {
        Some(FnArg::Receiver(receiver)) => receiver
            .reference
            .as_ref()
            .and_then(|(_, lifetime)| lifetime.as_ref())
            .filter(|lifetime| lifetime.ident != "_")
            .map(|lifetime| lifetime.ident.clone()),
        _ => None,
    };

    // --- Determine receiver ---
    let receiver = if raw_handle {
        Receiver::None
    } else {
        match sig.inputs.first() {
            Some(FnArg::Receiver(r)) => {
                if r.reference.is_none() {
                    Receiver::Value
                } else if r.mutability.is_some() {
                    Receiver::Mut
                } else {
                    Receiver::Ref
                }
            }
            _ if self_ty.is_some() => Receiver::None, // static method in exported impl
            _ if self_ty.is_none() && !has_default => Receiver::None, // free function
            _ => return Ok(None),                     // trait method without receiver — skip
        }
    };

    // Skip receiver or raw_handle's first param (the handle pointer)
    let skip_n = if receiver != Receiver::None || raw_handle {
        1
    } else {
        0
    };

    // --- Parse params ---
    let params: Vec<_> = sig
        .inputs
        .iter()
        .skip(skip_n)
        .filter_map(|arg| {
            let FnArg::Typed(pt) = arg else { return None };
            let Pat::Ident(pi) = &*pt.pat else {
                return None;
            };
            Some((pi, pt))
        })
        .map(|(pi, pt)| -> syn::Result<ParamInfo> {
            // Unwrap reference for impl Trait detection:
            // `&mut impl PushStr` → inner type is `impl PushStr`, ref_kind = "mut"
            let (inner_ty, impl_trait_ref_kind) = match &*pt.ty {
                Type::Reference(r) => {
                    let rk = if r.mutability.is_some() { "mut" } else { "ref" };
                    (&*r.elem, rk)
                }
                Type::Paren(p) => (&*p.elem, "value"),
                other => (other, "value"),
            };
            let inner_ty = match inner_ty {
                Type::Paren(p) => &*p.elem,
                other => other,
            };

            // Parse #[ffier(...)] attrs on the parameter
            let param_attrs = parse_ffier_param_attrs(&pt.attrs)?;
            if param_attrs.foreign_crate.is_some() && param_attrs.foreign_c_name.is_none() {
                return Err(syn::Error::new_spanned(
                    &pt.pat,
                    "#[ffier(foreign = ...)] requires c_name = \"...\" \
                     (the C typedef name for the foreign handle type)",
                ));
            }
            let foreign_crate_tokens = param_attrs.foreign_crate.as_ref().map(|p| quote! { #p });

            if let Some(info) = extract_impl_trait_info(inner_ty)? {
                let dispatch = param_attrs.dispatch.unwrap_or_else(|| "auto".to_string());
                return Ok(ParamInfo {
                    name: pi.ident.clone(),
                    kind: ParamKind::ImplTrait {
                        trait_name: info.trait_name,
                        dispatch,
                        ref_kind: impl_trait_ref_kind.to_string(),
                        trait_lifetime_args: info.trait_lifetime_args,
                        lifetime_bounds: info.lifetime_bounds,
                        ref_lifetime: match &*pt.ty {
                            Type::Reference(r) => {
                                r.lifetime.as_ref().map(|lt| lt.ident.to_string())
                            }
                            _ => None,
                        },
                    },
                    types: Some(TypePair {
                        bridge: quote! { *mut core::ffi::c_void },
                        rust: quote! { *mut core::ffi::c_void },
                        foreign_crate: None,
                        foreign_c_name: None,
                    }),
                });
            }

            // Replace Self with concrete type — lifetime-erased for bridge,
            // lifetime-preserving for rust (client codegen needs real lifetimes).
            let param_ty_bridge = match self_ty {
                Some(sty) => {
                    let static_ty = erase_lifetimes(sty);
                    replace_self_type(&pt.ty, &static_ty)
                }
                None => (*pt.ty).clone(),
            };
            let param_ty_rust = match self_ty {
                Some(sty) => replace_self_type(&pt.ty, sty),
                None => (*pt.ty).clone(),
            };

            if (is_str_slice(&param_ty_bridge)
                || is_handle_slice(&param_ty_bridge)
                || primitive_slice_elem(&param_ty_bridge).is_some())
                && let Type::Reference(reference) = &param_ty_rust
                && let Some(lifetime) = reference
                    .lifetime
                    .iter()
                    .chain(match &*reference.elem {
                        Type::Slice(slice) => match &*slice.elem {
                            Type::Reference(inner) => inner.lifetime.as_ref(),
                            _ => None,
                        },
                        _ => None,
                    })
                    .find(|lifetime| lifetime.ident != "_")
            {
                return Err(syn::Error::new_spanned(
                    lifetime,
                    "ffier explicit lifetimes on non-byte slice parameters are unsupported",
                ));
            }

            if is_str_slice(&param_ty_bridge) {
                return Ok(ParamInfo {
                    name: pi.ident.clone(),
                    kind: ParamKind::StrSlice,
                    types: None,
                });
            }

            if let Some(elem_ty) = primitive_slice_elem(&param_ty_bridge) {
                let elem_bridge = ctx.bridge_tokens(elem_ty);
                let elem_rust = match self_ty {
                    Some(sty) => {
                        let replaced = replace_self_type(elem_ty, sty);
                        quote! { #replaced }
                    }
                    None => {
                        quote! { #elem_ty }
                    }
                };
                return Ok(ParamInfo {
                    name: pi.ident.clone(),
                    kind: ParamKind::PrimitiveSlice,
                    types: Some(TypePair {
                        bridge: elem_bridge,
                        rust: elem_rust,
                        foreign_crate: foreign_crate_tokens.clone(),
                        foreign_c_name: param_attrs.foreign_c_name.clone(),
                    }),
                });
            }

            if is_handle_slice(&param_ty_bridge) {
                // Extract the inner type T from &[&T] for bridge resolution
                let Type::Reference(ref_ty) = &param_ty_bridge else {
                    unreachable!()
                };
                let Type::Slice(sl) = &*ref_ty.elem else {
                    unreachable!()
                };
                let Type::Reference(inner_ref) = &*sl.elem else {
                    unreachable!()
                };
                let inner_ty = &*inner_ref.elem;
                let elem_bridge = ctx.bridge_tokens(inner_ty);
                let elem_rust = match self_ty {
                    Some(sty) => {
                        let replaced = replace_self_type(inner_ty, sty);
                        quote! { #replaced }
                    }
                    None => {
                        quote! { #inner_ty }
                    }
                };
                return Ok(ParamInfo {
                    name: pi.ident.clone(),
                    kind: ParamKind::HandleSlice,
                    types: Some(TypePair {
                        bridge: elem_bridge,
                        rust: elem_rust,
                        foreign_crate: foreign_crate_tokens.clone(),
                        foreign_c_name: param_attrs.foreign_c_name.clone(),
                    }),
                });
            }

            let bridge = ctx.bridge_tokens(&param_ty_bridge);
            Ok(ParamInfo {
                name: pi.ident.clone(),
                kind: ParamKind::Regular,
                types: Some(TypePair {
                    bridge,
                    rust: quote! { #param_ty_rust },
                    foreign_crate: foreign_crate_tokens,
                    foreign_c_name: param_attrs.foreign_c_name,
                }),
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    // --- Parse return type ---
    let self_ty_static = self_ty.map(erase_lifetimes);
    let foreign_return_tokens = foreign_return.map(|p| quote! { #p });
    let foreign_ret_c_name = foreign_return_c_name.map(|s| s.to_string());

    // Builder detection: method returns Self (only for exported impls, requires a receiver)
    let has_receiver = receiver != Receiver::None;
    let is_builder = if let (Some(sty_static), true) = (&self_ty_static, has_receiver) {
        match &sig.output {
            ReturnType::Default => false,
            ReturnType::Type(_, ty) => {
                let ty = &replace_self_type(ty, sty_static);
                is_self_return(ty, sty_static)
                    || extract_result_types(ty)
                        .is_some_and(|(ok, _)| is_self_return(&ok, sty_static))
            }
        }
    } else {
        false
    };

    let ret = match &sig.output {
        ReturnType::Default => ReturnKind::Void,
        ReturnType::Type(_, ty) => {
            // Bridge path: Self replaced with lifetime-erased type
            let ty_bridge = match &self_ty_static {
                Some(sty) => replace_self_type(ty, sty),
                None => (**ty).clone(),
            };
            // Rust path: Self replaced with original type (lifetimes preserved)
            let ty_rust = match self_ty {
                Some(sty) => replace_self_type(ty, sty),
                None => (**ty).clone(),
            };

            if is_builder && extract_result_types(&ty_bridge).is_none() {
                // Builder returning Self → void in C
                ReturnKind::Void
            } else if let Some(inner_ty) = handle_slice_elem(&ty_bridge) {
                // &[&T] or &[T] where T is an exported handle
                let elem_bridge = ctx.bridge_tokens(inner_ty);
                let inner_ty_rust =
                    handle_slice_elem(&ty_rust).expect("rust type should also be a handle slice");
                // Detect &[T] (direct) vs &[&T] (indirect) by checking
                // whether the slice element is a reference.
                let Type::Reference(ref_ty) = &ty_bridge else {
                    unreachable!()
                };
                let Type::Slice(sl) = &*ref_ty.elem else {
                    unreachable!()
                };
                let direct = !matches!(&*sl.elem, Type::Reference(_));
                ReturnKind::HandleSlice {
                    types: TypePair {
                        bridge: elem_bridge,
                        rust: quote! { #inner_ty_rust },
                        foreign_crate: foreign_return_tokens.clone(),
                        foreign_c_name: foreign_ret_c_name.clone(),
                    },
                    direct,
                }
            } else if let Some((ok_bridge, err)) = extract_result_types(&ty_bridge) {
                let err_ident = type_ident_name(&err);
                let ok_rust = extract_result_types(&ty_rust).map(|(ok, _)| ok);
                let ok_pair = if is_unit_type(&ok_bridge)
                    || (is_builder
                        && self_ty_static
                            .as_ref()
                            .is_some_and(|sty| is_self_return(&ok_bridge, sty)))
                {
                    None
                } else if raw_handle {
                    let erased = erase_lifetimes(&ok_bridge);
                    let rust = ok_rust.as_ref().unwrap_or(&ok_bridge);
                    Some(TypePair {
                        bridge: quote! { #erased },
                        rust: quote! { #rust },
                        foreign_crate: foreign_return_tokens.clone(),
                        foreign_c_name: foreign_ret_c_name.clone(),
                    })
                } else {
                    let bridge = ctx.bridge_tokens(&ok_bridge);
                    let rust = ok_rust.as_ref().unwrap_or(&ok_bridge);
                    Some(TypePair {
                        bridge,
                        rust: quote! { #rust },
                        foreign_crate: foreign_return_tokens.clone(),
                        foreign_c_name: foreign_ret_c_name.clone(),
                    })
                };
                ReturnKind::Result {
                    ok: ok_pair,
                    err_ident,
                }
            } else if raw_handle {
                let erased = erase_lifetimes(&ty_bridge);
                ReturnKind::Value(TypePair {
                    bridge: quote! { #erased },
                    rust: quote! { #ty_rust },
                    foreign_crate: foreign_return_tokens.clone(),
                    foreign_c_name: foreign_ret_c_name.clone(),
                })
            } else {
                let bridge = ctx.bridge_tokens(&ty_bridge);
                ReturnKind::Value(TypePair {
                    bridge,
                    rust: quote! { #ty_rust },
                    foreign_crate: foreign_return_tokens,
                    foreign_c_name: foreign_ret_c_name,
                })
            }
        }
    };

    // The schema records lifetime names, not constraints. Reject constraints
    // rather than expose a safe client signature with weaker requirements.
    // `Self: Sized` is the one exception: it only relaxes object safety of the
    // Rust trait and imposes nothing on callers, so a client signature without
    // it is not weaker. Generated FFI functions are per-concrete-type, so the
    // bound carries no information the schema would need to represent. Every
    // other where clause constrains a caller-visible type or lifetime and
    // would silently weaken the generated client if dropped.
    let supported_where = sig.generics.where_clause.is_none() || sized_self_where(sig);
    if sig.generics.lifetimes().any(|lt| !lt.bounds.is_empty()) || !supported_where {
        return Err(syn::Error::new_spanned(
            &sig.generics,
            "ffier exported method lifetime bounds and where clauses are unsupported",
        ));
    }

    // --- Original rust_ret for client codegen ---
    let rust_ret = match &sig.output {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => {
            let replaced = match self_ty {
                Some(sty) => replace_self_type(ty, sty),
                None => (**ty).clone(),
            };
            Some(quote! { #replaced })
        }
    };

    if params.iter().any(|param| {
        matches!(&param.kind,
        ParamKind::ImplTrait { ref_kind, .. } if ref_kind != "value")
    }) && let ReturnType::Type(_, ty) = &sig.output
    {
        struct BorrowedReturn(bool);
        impl VisitMut for BorrowedReturn {
            fn visit_lifetime_mut(&mut self, _: &mut syn::Lifetime) {
                self.0 = true;
            }
            fn visit_type_reference_mut(&mut self, _: &mut syn::TypeReference) {
                self.0 = true;
            }
        }
        let mut return_type = match self_ty {
            Some(sty) => replace_self_type(ty, sty),
            None => (**ty).clone(),
        };
        let mut borrowed = BorrowedReturn(false);
        borrowed.visit_type_mut(&mut return_type);
        if borrowed.0 {
            return Err(syn::Error::new_spanned(
                &sig.output,
                "ffier borrowed callbacks with lifetime-bearing returns are unsupported; callback handles are call-scoped",
            ));
        }
    }

    Ok(Some(MethodInfo {
        name: sig.ident.clone(),
        receiver,
        receiver_lifetime,
        params,
        ret,
        ffi_name: String::new(),
        is_builder,
        method_lifetimes: sig
            .generics
            .lifetimes()
            .map(|lt| lt.lifetime.ident.clone())
            .collect(),
        doc_lines: extract_doc_comments(attrs),
        rust_ret,
        has_default,
        index: 0,
        raw_handle,
        cfg_predicates: extract_cfg_predicates(attrs)?,
    }))
}

/// Whether the method is from a trait definition or a concrete impl.
#[derive(Clone, Copy)]
pub(crate) enum MethodMetaKind {
    /// Trait definition method — carries index/has_default/raw_handle.
    Definition,
    /// Concrete method (exported struct impl or trait impl) — carries ffi_name/is_builder.
    Impl,
}

/// Emit metadata tokens for a method list.
pub(crate) fn emit_method_meta(
    methods: &[MethodInfo],
    ctx: MethodMetaKind,
) -> Vec<proc_macro2::TokenStream> {
    methods
        .iter()
        .map(|m| emit_one_method_meta(m, ctx))
        .collect()
}

fn emit_one_method_meta(m: &MethodInfo, ctx: MethodMetaKind) -> proc_macro2::TokenStream {
    let mname = &m.name;
    let doc_tokens: Vec<_> = m.doc_lines.iter().map(|d| quote! { #d }).collect();

    let receiver_ident = match m.receiver {
        Receiver::None => format_ident!("none"),
        Receiver::Ref => format_ident!("r#ref"),
        Receiver::Mut => format_ident!("r#mut"),
        Receiver::Value => format_ident!("value"),
    };
    let receiver_lifetime: Vec<_> = m.receiver_lifetime.iter().collect();

    let method_lt_idents: Vec<_> = m
        .method_lifetimes
        .iter()
        .map(|lt| format_ident!("{}", lt))
        .collect();

    let param_tokens: Vec<_> = m.params.iter().map(|p| {
        let id = &p.name;
        let kind_tokens = match &p.kind {
            ParamKind::Regular => quote! { regular },
            ParamKind::StrSlice => quote! { str_slice },
            ParamKind::HandleSlice => quote! { handle_slice },
            ParamKind::PrimitiveSlice => quote! { primitive_slice },
            ParamKind::ImplTrait { trait_name, dispatch, ref_kind, trait_lifetime_args, lifetime_bounds, ref_lifetime } => {
                let dispatch_ident = format_ident!("{dispatch}");
                let ref_kind_ident = match ref_kind.as_str() {
                    "ref" => format_ident!("r#ref"),
                    "mut" => format_ident!("r#mut"),
                    other => format_ident!("{other}"),
                };
                let lt_idents: Vec<_> = trait_lifetime_args.iter().map(|lt| format_ident!("{lt}")).collect();
                let bounds: Vec<_> = lifetime_bounds.iter().map(|lt| format_ident!("{lt}")).collect();
                let reference: Vec<_> = ref_lifetime.iter().map(|lt| format_ident!("{lt}")).collect();
                quote! { impl_trait, trait_name = #trait_name, dispatch = #dispatch_ident, ref_kind = #ref_kind_ident, trait_lifetime_args = [#(#lt_idents),*], lifetime_bounds = [#(#bounds),*], ref_lifetime = [#(#reference),*] }
            }
        };
        let type_tokens = if p.is_impl_trait() {
            quote! {}
        } else {
            match &p.types {
                Some(tp) => {
                    let bt = &tp.bridge;
                    let rt = &tp.rust;
                    let fc = tp.foreign_crate.as_ref().map(|fc| quote! { foreign_crate = (#fc), });
                    let fcn = tp.foreign_c_name.as_ref().map(|n| quote! { foreign_c_name = (#n), });
                    quote! { bridge_type = (#bt), rust_type = (#rt), #fc #fcn }
                }
                None => quote! {},
            }
        };
        quote! { { name = #id, kind = #kind_tokens, #type_tokens } }
    }).collect();

    let ret_tokens = match &m.ret {
        ReturnKind::Void => quote! { void },
        ReturnKind::Value(tp) => {
            let bt = &tp.bridge;
            let rt = &tp.rust;
            let fc = tp
                .foreign_crate
                .as_ref()
                .map(|fc| quote! { foreign_crate = (#fc), });
            let fcn = tp
                .foreign_c_name
                .as_ref()
                .map(|n| quote! { foreign_c_name = (#n), });
            quote! { value(bridge_type = (#bt), rust_type = (#rt), #fc #fcn) }
        }
        ReturnKind::Result { ok, err_ident } => {
            let ok_tokens = match ok {
                None => quote! { ok = void },
                Some(tp) => {
                    let bt = &tp.bridge;
                    let rt = &tp.rust;
                    let fc = tp
                        .foreign_crate
                        .as_ref()
                        .map(|fc| quote! { foreign_crate = (#fc), });
                    let fcn = tp
                        .foreign_c_name
                        .as_ref()
                        .map(|n| quote! { foreign_c_name = (#n), });
                    quote! { ok = some(bridge_type = (#bt), rust_type = (#rt), #fc #fcn) }
                }
            };
            quote! { result(#ok_tokens, err_ident = #err_ident,) }
        }
        ReturnKind::HandleSlice { types: tp, direct } => {
            let bt = &tp.bridge;
            let rt = &tp.rust;
            let fc = tp
                .foreign_crate
                .as_ref()
                .map(|fc| quote! { foreign_crate = (#fc), });
            let fcn = tp
                .foreign_c_name
                .as_ref()
                .map(|n| quote! { foreign_c_name = (#n), });
            if *direct {
                quote! { direct_handle_slice(bridge_type = (#bt), rust_type = (#rt), #fc #fcn) }
            } else {
                quote! { handle_slice(bridge_type = (#bt), rust_type = (#rt), #fc #fcn) }
            }
        }
    };

    let rust_ret_tokens = match &m.rust_ret {
        Some(rt) => quote! { rust_ret = (#rt), },
        None => quote! { rust_ret = (()), },
    };

    let context_tokens = match ctx {
        MethodMetaKind::Definition => {
            let has_default = m.has_default;
            let index = m.index;
            let raw_handle = m.raw_handle;
            quote! {
                method_kind = definition,
                has_default = #has_default,
                index = #index,
                raw_handle = #raw_handle,
            }
        }
        MethodMetaKind::Impl => {
            let ffi_name = &m.ffi_name;
            let is_builder = m.is_builder;
            quote! {
                method_kind = r#impl,
                ffi_name = #ffi_name,
                is_builder = #is_builder,
            }
        }
    };

    let cfg_tokens = match combine_cfg_predicates(&m.cfg_predicates) {
        Some(pred) => quote! { cfg = (#pred), },
        None => quote! {},
    };

    quote! {
        {
            name = #mname,
            doc = [#(#doc_tokens),*],
            receiver = #receiver_ident,
            receiver_lifetime = [#(#receiver_lifetime),*],
            #context_tokens
            method_lifetimes = [#(#method_lt_idents),*],
            params = [#(#param_tokens),*],
            ret = #ret_tokens,
            #rust_ret_tokens
            #cfg_tokens
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(sig: syn::Signature, self_ty: Option<&Type>) -> syn::Result<Option<MethodInfo>> {
        parse_method_sig(
            &sig,
            &[],
            &mut AliasContext::new(format_ident!("helper")),
            self_ty,
            false,
            false,
            None,
            None,
        )
    }

    fn assert_accepted(sig: syn::Signature) {
        let method = parse(sig, None).expect("signature rejected");
        assert!(method.is_some(), "signature skipped");
    }

    fn assert_rejected(sig: syn::Signature) {
        let result = parse(sig, None);
        assert!(result.is_err(), "signature accepted");
        assert!(
            result.err().unwrap().to_string().contains("where clauses"),
            "unexpected diagnostic"
        );
    }

    #[test]
    fn accepts_self_sized_on_trait_method() {
        assert_accepted(syn::parse_quote! {
            fn probe(&self) -> i32
            where
                Self: Sized
        });
    }

    #[test]
    fn accepts_self_sized_on_exported_impl_method() {
        let self_ty: Type = syn::parse_quote!(Widget);
        let method = parse(
            syn::parse_quote! {
                fn probe(&self) -> i32
                where
                    Self: Sized
            },
            Some(&self_ty),
        )
        .expect("signature rejected");
        assert!(method.is_some(), "signature skipped");
    }

    #[test]
    fn rejects_other_self_bounds() {
        assert_rejected(syn::parse_quote! {
            fn probe(&self) -> i32
            where
                Self: Clone
        });
    }

    #[test]
    fn rejects_other_where_predicates() {
        assert_rejected(syn::parse_quote! {
            fn probe<T>(&self) -> i32
            where
                T: 'static
        });
    }

    #[test]
    fn rejects_extra_predicates_beside_self_sized() {
        assert_rejected(syn::parse_quote! {
            fn probe(&self) -> i32
            where
                Self: Sized,
                Self: Clone
        });
    }

    #[test]
    fn rejects_lifetime_bounds_on_the_sized_predicate() {
        assert_rejected(syn::parse_quote! {
            fn probe(&self) -> i32
            where
                Self: Sized + 'static
        });
    }
}
