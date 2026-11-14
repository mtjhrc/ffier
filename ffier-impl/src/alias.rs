//! Definition-site type aliases and lifetime-preserving signature rewriting.

use quote::{format_ident, quote};
use syn::{GenericArgument, PathArguments, Type, visit_mut::VisitMut};

use crate::cfg::{combine_cfg_predicates, extract_cfg_predicates};
use crate::meta::{self, erase_lifetimes};
use crate::types::{is_box_str, is_builtin_type_path, is_option, is_primitive, option_inner};

/// Collect the lifetime occurrences of a type in AST order.
///
/// The position of each occurrence defines an alias formal parameter, so a
/// type that repeats one lifetime (`Handle<'a, 'a>`) gets two formals while
/// independent lifetimes (`Handle<'a, 'b>`) also get two. Both cases erase to
/// the same shape, which is what lets one alias item serve every instantiation.
fn alias_lifetime_slots(ty: &Type) -> Vec<syn::Lifetime> {
    struct Collect<'a>(&'a mut Vec<syn::Lifetime>);
    impl VisitMut for Collect<'_> {
        fn visit_lifetime_mut(&mut self, lt: &mut syn::Lifetime) {
            self.0.push(lt.clone());
        }
    }
    let mut slots = Vec::new();
    let mut ty = ty.clone();
    Collect(&mut slots).visit_type_mut(&mut ty);
    slots
}

/// Build the formal lifetime parameters an alias for `ty` declares.
fn alias_formals(slots: &[syn::Lifetime]) -> Vec<syn::Lifetime> {
    (0..slots.len())
        .map(|i| {
            syn::Lifetime::new(
                &format!("'{ALIAS_LIFETIME_PREFIX}{i}"),
                proc_macro2::Span::call_site(),
            )
        })
        .collect()
}

const ALIAS_LIFETIME_PREFIX: &str = "__ffier_lt";

/// Insert an explicit `'_` where the source omitted a lifetime argument.
///
/// `BorrowedFd` means `BorrowedFd<'_>`. Normalizing once keeps every later step
/// uniform: an anonymous lifetime is then just an occurrence, like a named one,
/// and the alias machinery never needs a second "was it omitted" code path.
fn normalize_omitted_lifetimes(ty: &Type) -> Type {
    struct Insert;
    impl VisitMut for Insert {
        fn visit_path_segment_mut(&mut self, segment: &mut syn::PathSegment) {
            if matches!(segment.arguments, PathArguments::None)
                && std_os_type_takes_lifetime(&segment.ident)
            {
                segment.arguments =
                    PathArguments::AngleBracketed(syn::AngleBracketedGenericArguments {
                        colon2_token: None,
                        lt_token: Default::default(),
                        args: [GenericArgument::Lifetime(syn::Lifetime::new(
                            "'_",
                            proc_macro2::Span::call_site(),
                        ))]
                        .into_iter()
                        .collect(),
                        gt_token: Default::default(),
                    });
            } else {
                syn::visit_mut::visit_path_segment_mut(self, segment);
            }
        }
    }
    let mut ty = ty.clone();
    Insert.visit_type_mut(&mut ty);
    ty
}

/// The standard OS types that take a lifetime the source may omit.
///
/// `OwnedFd`, `RawFd`, and `RawHandle` carry no lifetime and are ordinary types.
/// This is the only place that needs to know which bare paths elide a lifetime;
/// it decides spelling, not codegen.
fn std_os_type_takes_lifetime(ident: &syn::Ident) -> bool {
    matches!(ident.to_string().as_str(), "BorrowedFd" | "BorrowedHandle")
}

/// Rewrite `ty` so each lifetime occurrence uses the formal at its position.
///
/// An anonymous `'_` is an occurrence like any other, so it is replaced by the
/// matching formal; an alias item cannot keep an anonymous lifetime.
fn alias_body(ty: &Type, formals: &[syn::Lifetime]) -> Type {
    struct Replace<'a> {
        formals: &'a [syn::Lifetime],
        next: usize,
    }
    impl VisitMut for Replace<'_> {
        fn visit_lifetime_mut(&mut self, lt: &mut syn::Lifetime) {
            if let Some(formal) = self.formals.get(self.next) {
                *lt = formal.clone();
            }
            self.next += 1;
        }
    }
    let mut ty = ty.clone();
    Replace { formals, next: 0 }.visit_type_mut(&mut ty);
    ty
}

/// Rewrite the parameter and return types of a baked trait-method signature so
/// they resolve at the registration site.
///
/// The signature keeps its own lifetimes and `impl Trait` structure; only
/// named types are replaced by definition-site aliases.
pub(crate) fn resolve_signature_types(
    sig: &mut syn::Signature,
    attrs: &[syn::Attribute],
    ctx: &mut AliasContext,
) {
    // Aliases registered below belong to this method, so they must carry its
    // predicate; otherwise a platform- or feature-exclusive method would emit
    // an alias gated by whichever method happened to be parsed last.
    ctx.current_cfg = combine_cfg_predicates(&extract_cfg_predicates(attrs).unwrap_or_default());
    for arg in &mut sig.inputs {
        let syn::FnArg::Typed(pat_type) = arg else {
            continue;
        };
        resolve_type(&mut pat_type.ty, ctx);
    }
    if let syn::ReturnType::Type(_, ty) = &mut sig.output {
        // `Result<T, E>` keeps its own spelling (it is a std prelude path, not a
        // definition-site type); only its success type is rewritten.
        if let Type::Path(tp) = &mut **ty
            && is_builtin_type_path(tp, "Result")
            && let Some(PathArguments::AngleBracketed(args)) =
                tp.path.segments.last_mut().map(|s| &mut s.arguments)
            && let Some(GenericArgument::Type(ok)) = args.args.first_mut()
        {
            resolve_type(ok, ctx);
        } else {
            resolve_type(ty, ctx);
        }
    }
}

/// Replace named types inside `ty` with definition-site aliases.
///
/// References, slices, `impl Trait`, and the standard containers keep their
/// structure so the generated signature stays exactly as the user wrote it.
fn resolve_type(ty: &mut Type, ctx: &mut AliasContext) {
    match ty {
        Type::Reference(reference) => resolve_type(&mut reference.elem, ctx),
        Type::Slice(slice) => resolve_type(&mut slice.elem, ctx),
        Type::Paren(paren) => resolve_type(&mut paren.elem, ctx),
        Type::Path(tp) if tp.path.is_ident("str") => {}
        Type::Path(tp) if is_option(tp) => {
            if let Some(PathArguments::AngleBracketed(args)) =
                tp.path.segments.last_mut().map(|s| &mut s.arguments)
                && let Some(GenericArgument::Type(inner)) = args.args.first_mut()
            {
                resolve_type(inner, ctx);
            }
        }
        Type::ImplTrait(impl_trait) => {
            for bound in &mut impl_trait.bounds {
                if let syn::TypeParamBound::Trait(bound) = bound
                    && let Some(PathArguments::AngleBracketed(args)) =
                        bound.path.segments.last_mut().map(|s| &mut s.arguments)
                {
                    for arg in &mut args.args {
                        if let GenericArgument::Type(inner) = arg {
                            resolve_type(inner, ctx);
                        }
                    }
                }
            }
        }
        Type::Path(_) if !is_primitive(ty) && !is_box_str_path(ty) => {
            *ty = ctx.alias_type(ty);
        }
        _ => {}
    }
}

/// `Box<str>` stays inline; it has a built-in `FfiType` impl.
fn is_box_str_path(ty: &Type) -> bool {
    matches!(ty, Type::Path(tp) if is_box_str(tp))
}

/// Tracks type aliases needed for cross-crate `$crate::` resolution in metadata macros.
pub(crate) struct AliasContext {
    types: Vec<Type>,
    aliases: Vec<syn::Ident>,
    counter: u32,
    pub(crate) helper_mod: proc_macro2::TokenStream,
    pub(crate) current_cfg: Option<proc_macro2::TokenStream>,
    alias_cfgs: Vec<Option<proc_macro2::TokenStream>>,
}

impl AliasContext {
    pub(crate) fn new(helper_mod: syn::Ident) -> Self {
        Self {
            types: Vec::new(),
            aliases: Vec::new(),
            counter: 0,
            helper_mod: quote! { #helper_mod },
            current_cfg: None,
            alias_cfgs: Vec::new(),
        }
    }

    /// Produce bridge_type tokens for any Rust type, including references.
    ///
    /// Recursively handles reference and slice types, producing tokens
    /// that resolve via `<T as FfiType>::CRepr` in the cdylib context.
    /// Primitives and `str` are emitted directly; everything else gets
    /// a `$crate::helper_mod::_TypeN` alias for cross-crate resolution.
    pub(crate) fn bridge_tokens(&mut self, ty: &Type) -> proc_macro2::TokenStream {
        match ty {
            Type::Reference(ref_ty) => {
                let inner = self.bridge_tokens(&ref_ty.elem);
                if ref_ty.mutability.is_some() {
                    quote! { &'static mut #inner }
                } else {
                    quote! { &'static #inner }
                }
            }
            // Raw pointers (*mut T, *const T) — recurse into the pointee.
            // The user must write fully qualified paths (e.g. `*mut core::ffi::c_void`).
            Type::Ptr(ptr_ty) => {
                let inner = self.bridge_tokens(&ptr_ty.elem);
                if ptr_ty.mutability.is_some() {
                    quote! { *mut #inner }
                } else {
                    quote! { *const #inner }
                }
            }
            Type::Slice(sl) => {
                let elem = self.bridge_tokens(&sl.elem);
                quote! { [#elem] }
            }
            Type::Path(tp) if tp.path.is_ident("str") => quote! { str },
            Type::Path(tp) if is_option(tp) => {
                let inner_ty = option_inner(tp);
                let inner = self.bridge_tokens(inner_ty);
                quote! { Option<#inner> }
            }
            Type::Path(tp) if is_box_str(tp) => quote! { Box<str> },
            _ => self.alias_tokens(ty),
        }
    }

    /// Get or create an alias for a non-reference, non-slice, non-keyword type.
    pub(crate) fn alias_tokens(&mut self, ty: &Type) -> proc_macro2::TokenStream {
        if is_primitive(ty) {
            return quote! { #ty };
        }
        // Normalize first so this path and `alias_type` register one alias for
        // an omitted-lifetime type rather than two.
        let ty = &normalize_omitted_lifetimes(ty);
        let alias = self.alias_ident(ty);
        let helper = &self.helper_mod;
        let args: Vec<syn::Lifetime> = alias_lifetime_slots(ty)
            .into_iter()
            .map(|lt| syn::Lifetime::new("'static", lt.apostrophe))
            .collect();
        if args.is_empty() {
            return quote! { $crate::#helper::#alias };
        }
        quote! { $crate::#helper::#alias <#(#args),*> }
    }

    /// Build the alias for `ty` as a parsed type, for signature rewriting.
    ///
    /// `alias_tokens` emits `$crate::...`, which is fine inside a macro body but
    /// cannot be parsed as a `syn::Type`; signatures need the AST form.
    fn alias_type(&mut self, ty: &Type) -> Type {
        let ty = &normalize_omitted_lifetimes(ty);
        let alias = self.alias_ident(ty);
        let helper = self.helper_mod.clone();
        // An omitted lifetime became an explicit `'_`, so it is passed through
        // and elides at the use site exactly as the source did.
        let args = alias_lifetime_slots(ty);
        let mut path: syn::Path = syn::parse_quote!(#helper::#alias);
        if !args.is_empty() {
            let last = path.segments.last_mut().expect("alias path has a segment");
            last.arguments = PathArguments::AngleBracketed(syn::AngleBracketedGenericArguments {
                colon2_token: None,
                lt_token: Default::default(),
                args: args.into_iter().map(GenericArgument::Lifetime).collect(),
                gt_token: Default::default(),
            });
        }
        syn::parse_quote!(#path)
    }

    /// Register a normalized type, merging predicates for an existing alias.
    fn alias_ident(&mut self, ty: &Type) -> syn::Ident {
        let key = erase_lifetimes(ty);
        if let Some(i) = self
            .types
            .iter()
            .position(|existing| erase_lifetimes(existing) == key)
        {
            self.alias_cfgs[i] = match (&self.alias_cfgs[i], &self.current_cfg) {
                (Some(a), Some(b)) if a.to_string() == b.to_string() => Some(a.clone()),
                (Some(a), Some(b)) => Some(quote! { any(#a, #b) }),
                _ => None,
            };
            return self.aliases[i].clone();
        }
        let alias = format_ident!("_Type{}", self.counter);
        self.counter += 1;
        self.types.push(ty.clone());
        self.aliases.push(alias.clone());
        self.alias_cfgs.push(self.current_cfg.clone());
        alias
    }

    /// Emit type aliases in a helper module one level below the definition site.
    ///
    /// Keep absolute paths intact, replace `self::` with `super::`, and
    /// prepend `super::` to other local paths.
    pub(crate) fn local_type_aliases(&self) -> Vec<proc_macro2::TokenStream> {
        self.local_type_aliases_with_native(None)
    }

    /// Keep aliases of the opaque self type tied to its definition-site identity.
    pub(crate) fn local_type_aliases_for_native(
        &self,
        native_ty: &Type,
        native_path: &proc_macro2::TokenStream,
    ) -> Vec<proc_macro2::TokenStream> {
        let erased = erase_lifetimes(native_ty);
        self.local_type_aliases_with_native(Some((&erased, native_path)))
    }

    fn local_type_aliases_with_native(
        &self,
        native: Option<(&Type, &proc_macro2::TokenStream)>,
    ) -> Vec<proc_macro2::TokenStream> {
        let mut aliases: Vec<_> = self
            .types
            .iter()
            .zip(self.aliases.iter())
            .zip(self.alias_cfgs.iter())
            .map(|((ty, alias), cfg)| {
                let cfg = meta::cfg_predicate_attr(cfg.as_ref());
                let mut erased = erase_lifetimes(ty);
                // The opaque self alias takes its parameters from the native
                // type, which keeps the lifetimes the erased form drops.
                if let Some((native_ty, native_path)) = native
                    && &erased == native_ty
                {
                    let slots = alias_lifetime_slots(native_ty);
                    let formals = alias_formals(&slots);
                    let generics = if formals.is_empty() {
                        quote! {}
                    } else {
                        quote! { <#(#formals),*> }
                    };
                    return quote! {
                        #[allow(private_interfaces)]
                        #cfg
                        pub type #alias #generics = #native_path #generics;
                    };
                }
                let ty = &normalize_omitted_lifetimes(ty);
                let slots = alias_lifetime_slots(ty);
                let formals = alias_formals(&slots);
                let generics = if formals.is_empty() {
                    quote! {}
                } else {
                    quote! { <#(#formals),*> }
                };
                erased = alias_body(ty, &formals);
                if is_absolute_type_path(&erased) {
                    quote! { #cfg pub type #alias #generics = #erased; }
                } else if let Type::Path(tp) = &mut erased
                    && tp.path.segments[0].ident == "self"
                {
                    tp.path.segments[0].ident =
                        syn::Ident::new("super", tp.path.segments[0].ident.span());
                    quote! { #cfg pub type #alias #generics = #erased; }
                } else {
                    quote! { #cfg pub type #alias #generics = super::#erased; }
                }
            })
            .collect();
        // Relative module paths and external crate paths have identical syntax.
        // Import the definition's scope so both resolve inside this helper.
        if !aliases.is_empty() {
            aliases.insert(0, quote! { #[allow(unused_imports)] use super::*; });
        }
        aliases
    }
}

/// Check if a type starts with `crate::`, `::`, or an external crate path
/// (e.g. `other_crate::Foo`).
/// Such types should NOT get `super::` prepended in helper module aliases.
fn is_absolute_type_path(ty: &Type) -> bool {
    let Type::Path(tp) = ty else { return false };
    if tp.path.leading_colon.is_some() || tp.path.segments[0].ident == "crate" {
        return true;
    }
    if tp.path.segments.len() <= 1 {
        return false;
    }
    let first = tp.path.segments.first().unwrap().ident.to_string();
    !matches!(first.as_str(), "self" | "super")
}
