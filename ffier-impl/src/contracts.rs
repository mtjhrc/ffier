//! Validation and compiler assertions for handle and trait contracts.

use ffier_schema::{LifetimeVariance, ThreadSafety};
use quote::{ToTokens, format_ident, quote};
use syn::{GenericArgument, ItemImpl, ItemTrait, PathArguments, Type, visit_mut::VisitMut};

use crate::export::ExportCfg;

/// Check actual compiler trait information, never guesses based on field syntax.
pub(crate) fn thread_safety_assertion(
    ty: &impl ToTokens,
    generics: &syn::Generics,
    thread_safety: ThreadSafety,
) -> proc_macro2::TokenStream {
    if thread_safety.is_empty() {
        return quote! {};
    }
    let bounds: Vec<syn::Path> = thread_safety
        .rust_bounds()
        .map(|bound| syn::parse_str(bound).expect("standard marker trait path"))
        .collect();
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    quote! {
        const _: () = {
            fn __ffier_check_thread_safety #impl_generics () #where_clause {
                fn require<T: ?Sized + #(#bounds)+*>() {}
                require::<#ty>();
            }
        };
    }
}

/// The client cannot express lifetime constraints or specialized opaque types.
/// Require a complete declaration for the same independent parameters we emit.
pub(crate) fn validate_handle_variance(
    input: &ItemImpl,
    cfg: &ExportCfg,
) -> syn::Result<Vec<LifetimeVariance>> {
    if input.generics.type_params().next().is_some()
        || input.generics.const_params().next().is_some()
        || input.generics.where_clause.is_some()
        || input.generics.lifetimes().any(|lt| !lt.bounds.is_empty())
    {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "ffier opaque exports require only unconstrained lifetime parameters",
        ));
    }
    let Type::Path(path) = &*input.self_ty else {
        return Err(syn::Error::new_spanned(
            &input.self_ty,
            "expected a named struct type",
        ));
    };
    let parameters: Vec<_> = input.generics.lifetimes().map(|lt| &lt.lifetime).collect();
    let arguments: Vec<_> = match &path.path.segments.last().unwrap().arguments {
        PathArguments::None => Vec::new(),
        PathArguments::AngleBracketed(args) => args.args.iter().collect(),
        _ => {
            return Err(syn::Error::new_spanned(
                &input.self_ty,
                "unsupported opaque type arguments",
            ));
        }
    };
    if arguments.len() != parameters.len()
        || !arguments
            .iter()
            .zip(&parameters)
            .all(|(arg, param)| matches!(arg, GenericArgument::Lifetime(lt) if lt == *param))
    {
        return Err(syn::Error::new_spanned(
            &input.self_ty,
            "ffier opaque exports require all lifetime parameters in declaration order, without specialized type arguments",
        ));
    }
    for (lifetime, _) in &cfg.lifetime_variance {
        if !parameters.contains(&lifetime) {
            return Err(syn::Error::new_spanned(
                lifetime,
                "ffier variance annotation names an unknown lifetime parameter",
            ));
        }
    }
    let mut variances = Vec::with_capacity(parameters.len());
    for lifetime in parameters {
        let Some((_, variance)) = cfg.lifetime_variance.iter().find(|(lt, _)| lt == lifetime)
        else {
            return Err(syn::Error::new_spanned(
                lifetime,
                format!(
                    "ffier opaque lifetime {lifetime} requires an explicit variance annotation: covariant({lifetime}), contravariant({lifetime}), or invariant({lifetime})"
                ),
            ));
        };
        variances.push(*variance);
    }
    Ok(variances)
}

/// Check one parameter at a time, preserving every other lifetime verbatim.
/// No native type appears in the well-formedness function's signature: otherwise
/// Rust's implied bounds could hide constraints and make coercions vacuous.
pub(crate) fn lifetime_variance_assertions(
    input: &ItemImpl,
    variances: &[LifetimeVariance],
) -> proc_macro2::TokenStream {
    if variances.is_empty() {
        return quote! {};
    }
    let parameters: Vec<&syn::Lifetime> =
        input.generics.lifetimes().map(|lt| &lt.lifetime).collect();
    let ty = &input.self_ty;
    let checks =
        parameters
            .iter()
            .zip(variances)
            .enumerate()
            .filter_map(|(index, (short, variance))| {
                if *variance == LifetimeVariance::Invariant {
                    return None;
                }
                let long = fresh_helper_lifetime(&parameters, index);
                struct ReplaceLifetime<'a>(&'a syn::Lifetime, &'a syn::Lifetime);
                impl VisitMut for ReplaceLifetime<'_> {
                    fn visit_lifetime_mut(&mut self, lt: &mut syn::Lifetime) {
                        if lt == self.0 {
                            *lt = self.1.clone();
                        }
                    }
                }
                let mut long_ty = (**ty).clone();
                ReplaceLifetime(short, &long).visit_type_mut(&mut long_ty);
                let (source, target) = match variance {
                    LifetimeVariance::Covariant => (long_ty, (**ty).clone()),
                    LifetimeVariance::Contravariant => ((**ty).clone(), long_ty),
                    LifetimeVariance::Invariant => unreachable!(),
                };
                // Parameter index, not lifetime text: the user may declare '_a,
                // which would collide with the reserved '__ffier_variance_long'
                // prefix and produce an unused-lifetime warning under
                // deny(warnings).
                let name = format_ident!("__ffier_check_{}_param_{}", variance.as_str(), index);
                Some(quote! {
                    #[allow(clippy::needless_lifetimes)]
                    fn #name<#(#parameters,)* #long: #short>(value: #source) -> #target { value }
                })
            });
    quote! {
        const _: () = {
            #[allow(clippy::extra_unused_lifetimes)]
            fn __ffier_check_variance_parameters<#(#parameters),*>() {
                let _: ::core::option::Option<#ty> = ::core::option::Option::None;
            }
            #(#checks)*
        };
    }
}

/// Only explicit standard marker paths are supported. A compiler assertion
/// also verifies unqualified names, which could otherwise shadow marker traits.
pub(crate) fn thread_safety_bound(bound: &syn::TypeParamBound) -> Option<ThreadSafety> {
    let syn::TypeParamBound::Trait(bound) = bound else {
        return None;
    };
    if bound.lifetimes.is_some()
        || !matches!(bound.modifier, syn::TraitBoundModifier::None)
        || bound
            .path
            .segments
            .iter()
            .any(|segment| !matches!(segment.arguments, PathArguments::None))
    {
        return None;
    }
    let segments: Vec<_> = bound
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let name = match segments.as_slice() {
        [name] => name,
        [root, module, name] if (root == "std" || root == "core") && module == "marker" => name,
        _ => return None,
    };
    match name.as_str() {
        "Send" => Some(ThreadSafety {
            send: true,
            sync: false,
        }),
        "Sync" => Some(ThreadSafety {
            send: false,
            sync: true,
        }),
        _ => None,
    }
}

pub(crate) fn trait_thread_safety(trait_item: &ItemTrait) -> ThreadSafety {
    trait_item
        .supertraits
        .iter()
        .filter_map(thread_safety_bound)
        .fold(ThreadSafety::default(), |mut guarantees, bound| {
            guarantees.send |= bound.send;
            guarantees.sync |= bound.sync;
            guarantees
        })
}

/// Pick a synthetic helper lifetime that never collides with a declared
/// parameter and never shares its text.
fn fresh_helper_lifetime(parameters: &[&syn::Lifetime], index: usize) -> syn::Lifetime {
    let mut long_name = format!("__ffier_variance_long_{index}");
    while parameters.iter().any(|lt| lt.ident == long_name) {
        long_name.push('_');
    }
    syn::Lifetime::new(&format!("'{long_name}"), proc_macro2::Span::call_site())
}
