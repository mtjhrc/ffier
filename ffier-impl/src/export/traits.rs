//! Export traits, dynamic dispatch forwarding, and callback metadata.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use ffier_schema::ThreadSafety;
use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{
    FnArg, ItemTrait, LitStr, Pat, Token, TraitItem, Type, parse::Parse, parse_macro_input,
    visit_mut::VisitMut,
};

use super::ExportCfg;
use crate::{
    MACRO_COUNTER,
    alias::{AliasContext, resolve_signature_types},
    attrs::{parse_ffier_method_attrs, strip_ffier_attrs},
    cfg::{self, combine_cfg_predicates, extract_cfg_predicates},
    contracts::{thread_safety_assertion, thread_safety_bound, trait_thread_safety},
    meta::camel_to_snake,
    reexport::reexport_helpers_with,
    signature::{
        MethodInfo, MethodMetaKind, ParamInfo, emit_method_meta, parse_method_sig, sized_self_where,
    },
};

pub(super) struct ImplementableArgs {
    /// Reserved vtable slot indices (retired methods). These slots are padded
    /// in the vtable struct to keep the layout stable.
    reserved: Vec<usize>,
    /// If true, the trait is foreign (defined in another crate). The macro
    /// will not emit the trait definition or `&mut dyn Trait` dispatch impl.
    foreign: bool,
    /// Optional blessing tag for well-known types (e.g. `"error_trait"`).
    bless: Option<String>,
    /// If true, no vtable wrapper type is generated. C callers cannot
    /// implement this trait — only concrete Rust implementors are dispatched.
    /// Useful for marker traits and traits with supertrait bounds.
    no_vtable: bool,
    /// `#[cfg(...)]` predicate for conditional compilation.
    export_cfg: ExportCfg,
}

impl Parse for ImplementableArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut reserved = Vec::new();
        let mut foreign = false;
        let mut bless = None;
        let mut no_vtable = false;
        let mut cfg = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;

            if ident == "prefix" {
                input.parse::<Token![=]>()?;
                let _lit: LitStr = input.parse()?;
            } else if ident == "reserved" {
                let content;
                syn::parenthesized!(content in input);
                while !content.is_empty() {
                    let lit: syn::LitInt = content.parse()?;
                    let slot = lit.base10_parse::<usize>()?;
                    // Reserved slots also raise the slot count, which the
                    // vtable header stores as u16.
                    if slot >= usize::from(u16::MAX) {
                        return Err(syn::Error::new_spanned(
                            lit,
                            "reserved vtable slot must leave room for the slot \
                             count to fit in u16, because the vtable header \
                             stores it as u16",
                        ));
                    }
                    reserved.push(slot);
                    let _ = content.parse::<Token![,]>();
                }
            } else if ident == "foreign" {
                foreign = true;
            } else if ident == "bless" {
                input.parse::<Token![=]>()?;
                let lit: LitStr = input.parse()?;
                bless = Some(lit.value());
            } else if ident == "no_vtable" {
                no_vtable = true;
            } else if ident == "cfg" {
                input.parse::<Token![=]>()?;
                let lit: LitStr = input.parse()?;
                cfg = Some(cfg::normalize_tokens(
                    &lit.parse::<proc_macro2::TokenStream>()?,
                )?);
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    "expected `prefix`, `reserved`, `foreign`, `bless`, `no_vtable`, or `cfg`",
                ));
            }
            let _ = input.parse::<Token![,]>();
        }

        Ok(Self {
            reserved,
            foreign,
            bless,
            no_vtable,
            export_cfg: ExportCfg {
                cfg,
                thread_safety: ThreadSafety::default(),
                lifetime_variance: Vec::new(),
            },
        })
    }
}

fn extract_vtable_methods(
    trait_item: &ItemTrait,
    reserved: &[usize],
    ctx: &mut AliasContext,
) -> syn::Result<Vec<MethodInfo>> {
    let mut methods = Vec::new();

    for item in &trait_item.items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        let has_default = method.default.is_some();
        let mattrs = parse_ffier_method_attrs(&method.attrs)?;
        if mattrs.skip {
            continue;
        }
        if let Some(mut m) = parse_method_sig(
            &method.sig,
            &method.attrs,
            ctx,
            None,
            has_default,
            mattrs.raw_handle,
            mattrs.foreign_return.as_ref(),
            mattrs.foreign_return_c_name.as_deref(),
        )? {
            let index = mattrs.index.ok_or_else(|| {
                syn::Error::new_spanned(
                    &method.sig.ident,
                    format!(
                        "vtable method `{}` is missing `#[ffier(index = N)]`",
                        method.sig.ident,
                    ),
                )
            })?;
            m.index = index;
            methods.push(m);
        }
    }

    // Validate: one slot may hold cfg-exclusive variants of one method, and
    // only under the same name. The name is what makes them the same slot, so a
    // name that repeats must also repeat its index.
    let mut slot_names: HashMap<usize, &syn::Ident> = HashMap::new();
    let mut name_slots: HashMap<&syn::Ident, usize> = HashMap::new();
    for m in &methods {
        if let Some(prev) = name_slots.insert(&m.name, m.index)
            && prev != m.index
        {
            return Err(syn::Error::new_spanned(
                &m.name,
                format!(
                    "`{name}` uses vtable indices {prev} and {index}; \
                     cfg-exclusive variants of one method must share an index",
                    name = m.name,
                    index = m.index,
                ),
            ));
        }
        if let Some(prev) = slot_names.insert(m.index, &m.name)
            && prev != &m.name
        {
            return Err(syn::Error::new_spanned(
                &m.name,
                format!(
                    "duplicate vtable index {}: both `{}` and `{}` use index {}; \
                     cfg-exclusive variants must keep the same method name",
                    m.index, prev, m.name, m.index,
                ),
            ));
        }
    }

    // Validate: no method uses a reserved index
    for m in &methods {
        if reserved.contains(&m.index) {
            return Err(syn::Error::new_spanned(
                &m.name,
                format!(
                    "vtable index {} is reserved (retired slot) but used by method `{}`",
                    m.index, m.name,
                ),
            ));
        }
    }

    Ok(methods)
}

fn emit_dyn_dispatch_impl(trait_item: &ItemTrait) -> proc_macro2::TokenStream {
    // A `where Self: Sized` method is not callable on a trait object, so a
    // forwarding impl cannot provide it. A default body already satisfies one,
    // but a required method would leave the impl incomplete.
    let sized_methods: Vec<&syn::TraitItemFn> = trait_item
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(method) if sized_self_where(&method.sig) => Some(method),
            _ => None,
        })
        .collect();
    if sized_methods.iter().any(|method| method.default.is_none()) {
        return quote! {};
    }
    let trait_name = &trait_item.ident;
    let mut generics = trait_item.generics.clone();
    generics
        .params
        .insert(0, syn::parse_quote!('__ffier_object));
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let (_, trait_generics, _) = trait_item.generics.split_for_impl();
    let method_impls: Vec<_> = trait_item
        .items
        .iter()
        .filter_map(|item| {
            let TraitItem::Fn(method) = item else {
                return None;
            };
            let sig = &method.sig;
            if !matches!(sig.inputs.first(), Some(FnArg::Receiver(_))) || sized_self_where(sig) {
                return None;
            }
            let name = &sig.ident;
            let params: Vec<_> = sig
                .inputs
                .iter()
                .skip(1)
                .filter_map(|arg| {
                    let FnArg::Typed(pt) = arg else { return None };
                    let Pat::Ident(pi) = &*pt.pat else {
                        return None;
                    };
                    Some(pi.ident.clone())
                })
                .collect();
            let lifetime_lint = sig
                .receiver()
                .and_then(|receiver| receiver.reference.as_ref())
                .and_then(|(_, lifetime)| lifetime.as_ref())
                .map(|_| quote! { #[allow(clippy::needless_lifetimes)] });
            // Cfg-exclusive trait methods must stay gated here too: this macro
            // sees the trait before cfg-stripping, so an ungated forward would
            // name a method the compiled trait may not have.
            let cfg_attrs: Vec<&syn::Attribute> = method
                .attrs
                .iter()
                .filter(|attr| attr.path().is_ident("cfg"))
                .collect();
            Some(quote! { #(#cfg_attrs)* #lifetime_lint #sig { (**self).#name(#(#params),*) } })
        })
        .collect();

    let thread_safety = trait_thread_safety(trait_item);
    let shared_impl = if (thread_safety.send && !thread_safety.sync)
        || trait_item.items.iter().any(|item| matches!(item,
            TraitItem::Fn(method) if matches!(method.sig.inputs.first(), Some(FnArg::Receiver(r)) if r.mutability.is_some()))) {
        quote! {}
    } else {
        quote! {
            impl #impl_generics #trait_name #trait_generics for &(dyn #trait_name #trait_generics + '__ffier_object) #where_clause {
                #(#method_impls)*
            }
        }
    };

    quote! {
        const _: () = {
            impl #impl_generics #trait_name #trait_generics for Box<dyn #trait_name #trait_generics + '__ffier_object> #where_clause {
                #(#method_impls)*
            }
            impl #impl_generics #trait_name #trait_generics for &mut (dyn #trait_name #trait_generics + '__ffier_object) #where_clause {
                #(#method_impls)*
            }
            #shared_impl
        };
    }
}

/// Generate owned and mutable borrowed dynamic dispatch forwarding.
///
/// `#[ffier::export]` on traits implies `#[ffier::dispatch]` — use this
/// annotation alone when you want dynamic dispatch fallback without
/// exporting the trait's vtable to C.
pub(crate) fn dispatch(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let trait_item = parse_macro_input!(item as ItemTrait);
    let original_trait = trait_item.clone();
    let dyn_impl = emit_dyn_dispatch_impl(&trait_item);

    let output = quote! {
        #original_trait
        #dyn_impl
    };

    output.into()
}

/// Find the vtable method a trait item describes.
///
/// Names repeat when cfg-exclusive variants share a slot, so the item's own cfg
/// predicates are part of its identity. Matching by name alone would classify an
/// ordinary variant as `raw_handle` whenever a sibling variant is.
fn vtable_method_matches<'a>(
    vtable_methods: &'a [MethodInfo],
    method: &syn::TraitItemFn,
) -> Option<&'a MethodInfo> {
    let predicates = extract_cfg_predicates(&method.attrs).ok()?;
    vtable_methods
        .iter()
        .find(|vm| vm.name == method.sig.ident && vm.cfg_predicates == predicates)
}

pub(super) fn implementable_inner(args: ImplementableArgs, trait_item: ItemTrait) -> TokenStream {
    let is_foreign = args.foreign;
    let original_trait = trait_item.clone();
    let export_cfg = &args.export_cfg;

    let trait_name = &trait_item.ident;
    let trait_name_str = trait_name.to_string();
    if let Some(where_clause) = &trait_item.generics.where_clause {
        return syn::Error::new_spanned(
            where_clause,
            "ffier exported trait where clauses are unsupported; declare Send/Sync as native supertraits",
        )
        .to_compile_error()
        .into();
    }
    let thread_safety = trait_thread_safety(&trait_item);
    // The schema records only Send/Sync and the generated client trait repeats
    // only those, so any other supertrait is dropped and cannot be reproduced.
    // Silently dropping one would weaken the client's contract, so vtable
    // traits reject it whenever it appears. A `no_vtable` trait is exempt:
    // dispatch is concrete-only, so nobody implements it through the ABI and
    // the missing bound cannot surprise a caller.
    if !args.no_vtable
        && trait_item
            .supertraits
            .iter()
            .any(|bound| thread_safety_bound(bound).is_none())
    {
        return syn::Error::new_spanned(&trait_item,
            "ffier exported traits support only Send/Sync supertraits; additional supertraits need no_vtable dispatch")
            .to_compile_error().into();
    }
    let ThreadSafety { send, sync } = thread_safety;
    if !thread_safety.is_empty()
        && (trait_item.generics.type_params().next().is_some()
            || trait_item.generics.const_params().next().is_some()
            || trait_item
                .generics
                .lifetimes()
                .any(|lt| !lt.bounds.is_empty()))
    {
        return syn::Error::new_spanned(
            &trait_item.generics,
            "ffier thread-safe traits support only unconstrained lifetime parameters",
        )
        .to_compile_error()
        .into();
    }
    let (_, check_trait_generics, _) = trait_item.generics.split_for_impl();
    let mut check_generics = trait_item.generics.clone();
    check_generics
        .params
        .push(syn::parse_quote!(__FfierType: #trait_name #check_trait_generics + ?Sized));
    let check_type: Type = syn::parse_quote!(__FfierType);
    let thread_safety_check = thread_safety_assertion(&check_type, &check_generics, thread_safety);
    if !args.no_vtable && !trait_item.generics.params.is_empty() {
        return syn::Error::new_spanned(&trait_item.generics,
            "ffier callback vtables with trait lifetime or type parameters are unsupported; use no_vtable and concrete dispatch")
            .to_compile_error().into();
    }
    let trait_snake = camel_to_snake(&trait_name_str);

    // Extract trait-level lifetime params (e.g. `<'a>` from `trait Snapshot<'a>`)
    let trait_lifetime_idents: Vec<&syn::Ident> = trait_item
        .generics
        .lifetimes()
        .map(|lt| &lt.lifetime.ident)
        .collect();

    let vtable_struct_name = format_ident!("{trait_name_str}Vtable");

    let wrapper_name = format_ident!("Vtable{trait_name_str}");
    let wrapper_c_handle_suffix = format!("Vtable{trait_name_str}");

    let helper_mod_name = format_ident!("_ffier_vtable_{trait_snake}");
    let mut ctx = AliasContext::new(helper_mod_name.clone());

    let vtable_methods = match extract_vtable_methods(&trait_item, &args.reserved, &mut ctx) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };
    if !args.no_vtable
        && let Some(method) = vtable_methods.iter().find(|method| {
            method
                .receiver_lifetime
                .as_ref()
                .is_some_and(|lifetime| lifetime == "static")
        })
    {
        return syn::Error::new_spanned(
            &method.name,
            "ffier callback methods with 'static receivers are unsupported; \
             use a named method lifetime or no_vtable",
        )
        .to_compile_error()
        .into();
    }
    let own_method_count = vtable_methods.len();
    let bless_tokens = match &args.bless {
        Some(s) => quote::quote! { #s },
        None => quote::quote! { none },
    };
    let no_vtable = args.no_vtable;
    let no_vtable_tokens = if no_vtable {
        quote::quote! { yes }
    } else {
        quote::quote! { no }
    };

    // Build trait path with 'static lifetimes for the wrapper impl
    let trait_generics = &trait_item.generics;
    let trait_ty_generics = {
        let mut g = trait_generics.clone();
        for lt in g.lifetimes_mut() {
            lt.lifetime = syn::Lifetime::new("'static", lt.lifetime.apostrophe);
        }
        let (_, tg, _) = g.split_for_impl();
        tg.to_token_stream()
    };
    // Erase lifetimes only on trait-level generics, not on method parameter types.
    // Method signatures keep their original lifetimes (anonymous `'_` for input
    // positions like `s: &str`) — erasing them to `'static` would make the impl
    // unsatisfiable for callers with shorter borrows.
    let trait_item_erased = {
        let mut t = trait_item.clone();
        struct Eraser;
        impl VisitMut for Eraser {
            fn visit_lifetime_mut(&mut self, lt: &mut syn::Lifetime) {
                *lt = syn::Lifetime::new("'static", lt.apostrophe);
            }
            // Don't descend into method signatures — keep their lifetimes as-is.
            fn visit_trait_item_fn_mut(&mut self, _: &mut syn::TraitItemFn) {}
        }
        Eraser.visit_item_trait_mut(&mut t);
        t
    };

    // Helper: generate the vtable call expression for a method (unwrapping Option).
    // `fallback` is Some(tokens) for defaulted methods, None for required methods.
    //
    // For defaulted methods, we first check whether the handle metadata carries
    // ffier's default-dispatch marker for this method index. If so, we skip
    // vtable dispatch and call the library default directly. This prevents
    // infinite re-entrancy when a client trait default calls through
    // self-dispatch.
    // Helper: generate the vtable call expression for a method.
    // `vtable_struct_ref` is the token stream referencing the vtable struct
    // (e.g. `PushStrVtable` for direct output, or `$crate::PushStrVtable`
    // for use inside a macro_rules! body that expands in another crate).
    // --- Default method extraction ---
    // For methods with default bodies, extract the body into a free helper function
    // and rewrite the trait's default to call it. This allows the VtableXxx impl's
    // None branch to also call the helper, preserving the default behavior when
    // the C side doesn't provide a function pointer.
    //
    // Visitor to replace `self` with `__self` in default method bodies.
    struct SelfReplacer;
    impl SelfReplacer {
        /// Replace `self` idents in a raw TokenStream (for macro bodies
        /// that syn::visit_mut doesn't traverse).
        fn replace_in_token_stream(
            &self,
            ts: proc_macro2::TokenStream,
        ) -> proc_macro2::TokenStream {
            ts.into_iter()
                .map(|tt| match tt {
                    proc_macro2::TokenTree::Ident(ref id) if id == "self" => {
                        proc_macro2::TokenTree::Ident(proc_macro2::Ident::new("__self", id.span()))
                    }
                    proc_macro2::TokenTree::Group(g) => {
                        let inner = self.replace_in_token_stream(g.stream());
                        let mut new_g = proc_macro2::Group::new(g.delimiter(), inner);
                        new_g.set_span(g.span());
                        proc_macro2::TokenTree::Group(new_g)
                    }
                    other => other,
                })
                .collect()
        }
    }
    impl VisitMut for SelfReplacer {
        fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
            if let syn::Expr::Path(ep) = expr
                && ep.qself.is_none()
                && ep.path.is_ident("self")
            {
                *expr = syn::parse_quote! { __self };
                return;
            }
            syn::visit_mut::visit_expr_mut(self, expr);
        }
        fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
            mac.tokens = self.replace_in_token_stream(mac.tokens.clone());
        }
    }

    // Foreign traits must not have default method bodies — the real defaults
    // live in the foreign crate and can't be replicated here.
    // Exception: raw_handle methods always need their default body because
    // they're not dispatched through the vtable.
    if is_foreign {
        for item in &trait_item.items {
            if let TraitItem::Fn(method) = item {
                let Some(vm) = vtable_method_matches(&vtable_methods, method) else {
                    continue;
                };
                if method.default.is_some() && !vm.raw_handle {
                    return syn::Error::new_spanned(
                        &method.sig.ident,
                        "foreign trait methods must not have default bodies \
                         (the defaults live in the foreign crate)",
                    )
                    .to_compile_error()
                    .into();
                }
            }
        }
    }

    let mut default_helpers: Vec<proc_macro2::TokenStream> = Vec::new();
    // Map from method name → helper fn ident (only for methods with defaults)
    let mut default_helper_names: HashMap<String, syn::Ident> = HashMap::new();
    // Predicates under which a helper exists, keyed by helper ident. Helper
    // names drop the method's cfg, so cfg-exclusive variants of one method share
    // a helper ident and each contributes a predicate. Empty means the helper
    // exists wherever the trait does.
    let mut default_helper_cfgs: HashMap<String, Vec<proc_macro2::TokenStream>> = HashMap::new();

    // Build modified trait: replace default bodies with helper calls
    // and strip #[ffier(index = N)] attributes (consumed by macro).
    let mut modified_trait = original_trait.clone();
    for item in &mut modified_trait.items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        // Strip all #[ffier(...)] from emitted method attrs (consumed by this macro)
        let vm = vtable_method_matches(&vtable_methods, method);
        strip_ffier_attrs(&mut method.attrs);
        let Some(vm) = vm else {
            // Rust-only methods retain their signatures and default bodies.
            continue;
        };
        let Some(default_block) = &method.default else {
            continue;
        };
        // raw_handle methods keep their default body as-is — they don't
        // go through the vtable helper extraction, and the VtableWrapper
        // uses the trait's default impl directly.
        if vm.raw_handle {
            continue;
        }

        let method_name = &method.sig.ident;
        let helper_name = format_ident!("__ffier_default_{trait_name}_{method_name}");
        default_helper_names.insert(method_name.to_string(), helper_name.clone());

        // Extract the default body, rewriting self → __self
        let mut body = default_block.clone();
        SelfReplacer.visit_block_mut(&mut body);

        // Preserve the receiver's borrow contract in the extracted helper.
        let mut helper_sig = method.sig.clone();
        // The helper is a free function over `impl Trait`, where `Self` does not
        // exist. Its only callers are the method bodies that carry the clause.
        if sized_self_where(&helper_sig) {
            helper_sig.generics.where_clause = None;
        }
        let receiver = helper_sig
            .receiver()
            .cloned()
            .expect("default method receiver");
        let mutability = &receiver.mutability;
        let helper_receiver = if let Some((_, lifetime)) = &receiver.reference {
            let lifetime = match lifetime {
                Some(lifetime) if lifetime.ident != "_" => lifetime.clone(),
                _ => {
                    let mut name = String::from("__ffier_receiver");
                    while helper_sig
                        .generics
                        .lifetimes()
                        .any(|lt| lt.lifetime.ident == name)
                    {
                        name.push('_');
                    }
                    let lifetime = syn::Lifetime::new(&format!("'{name}"), method_name.span());
                    helper_sig
                        .generics
                        .params
                        .insert(0, syn::parse_quote!(#lifetime));
                    lifetime
                }
            };
            // A method's elided output lifetimes refer to its receiver even
            // when other parameters borrow. Free helpers need that tie named.
            struct ReceiverElision<'a>(&'a syn::Lifetime);
            impl VisitMut for ReceiverElision<'_> {
                fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
                    if reference.lifetime.is_none() {
                        reference.lifetime = Some(self.0.clone());
                    }
                    syn::visit_mut::visit_type_reference_mut(self, reference);
                }
                fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
                    if lifetime.ident == "_" {
                        *lifetime = self.0.clone();
                    }
                }
            }
            ReceiverElision(&lifetime).visit_return_type_mut(&mut helper_sig.output);
            syn::parse_quote! {
                __self: &#lifetime #mutability (impl #trait_name + ?Sized)
            }
        } else {
            syn::parse_quote! { #mutability __self: impl #trait_name }
        };
        // Remove the receiver and add __self parameter
        let helper_params: Vec<_> = helper_sig
            .inputs
            .iter()
            .filter(|arg| !matches!(arg, syn::FnArg::Receiver(_)))
            .cloned()
            .collect();
        helper_sig.inputs = syn::punctuated::Punctuated::new();
        helper_sig.inputs.push(helper_receiver);
        for p in helper_params {
            helper_sig.inputs.push(p);
        }
        helper_sig.ident = helper_name.clone();

        // Cfg-exclusive variants of one method would otherwise generate the same
        // helper name twice. Only the active variant's helper is compiled, so the
        // name-keyed lookup stays unambiguous.
        let cfg_attrs: Vec<&syn::Attribute> = method
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("cfg"))
            .collect();
        let cfg_metas: Vec<syn::Meta> = cfg_attrs
            .iter()
            .map(|attr| {
                attr.parse_args_with(cfg::parse_meta)
                    .expect("cfg attributes were validated when parsing the trait")
            })
            .collect();
        // A variant with no predicate exists wherever the trait does, so it
        // contributes `all()`: an unguarded reexport. Without this, a
        // same-named gated variant could leave the map holding only a false
        // predicate and the unconditional helper would lose its reexport.
        let pred = combine_cfg_predicates(&cfg_metas).unwrap_or_else(|| quote!(all()));
        default_helper_cfgs
            .entry(helper_name.to_string())
            .or_default()
            .push(pred);
        default_helpers.push(quote! {
            #(#cfg_attrs)*
            #[doc(hidden)]
            #[allow(clippy::needless_lifetimes)]
            pub #helper_sig #body
        });

        // Rewrite the trait's default body to call the helper
        let params_pass: Vec<_> = method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| {
                if let syn::FnArg::Typed(pat_type) = arg
                    && let syn::Pat::Ident(pi) = &*pat_type.pat
                {
                    return Some(pi.ident.clone());
                }
                None
            })
            .collect();
        method.default = Some(syn::parse_quote! {
            { #helper_name(self #(, #params_pass)*) }
        });
    }

    // --- Generate VtableXxx method impls ---
    // These are used inside the @on_library_export macro arm. The vtable struct is
    // a sibling (also in @on_library_export), so bare names resolve correctly.

    // Count virtual method slots, including reserved gaps but excluding drop.
    // An empty trait has zero slots, unlike a trait with method index 0.
    let num_slots = vtable_methods
        .iter()
        .map(|m| m.index)
        .chain(args.reserved.iter().copied())
        .max()
        .map_or(0, |index| index + 1);

    // --- Metadata emission ---
    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{trait_snake}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{trait_name}");

    let vtable_method_meta = emit_method_meta(&vtable_methods, MethodMetaKind::Definition);

    // Bake method signatures for the vtable proc macro. These are the
    // original trait method signatures whose parameter and return types are
    // resolved to definition-site aliases, so a type imported privately by the
    // defining crate still names correctly at the registration site.
    let vtable_method_sigs: Vec<proc_macro2::TokenStream> = trait_item_erased
        .items
        .clone()
        .into_iter()
        .filter_map(|item| {
            let TraitItem::Fn(method) = item else {
                return None;
            };
            let name = &method.sig.ident;
            // Cfg-exclusive variants share a name, so pair them by name and
            // predicate: that pair is unique.
            let predicates = extract_cfg_predicates(&method.attrs).unwrap_or_default();
            let vm = vtable_methods
                .iter()
                .find(|v| v.name == *name && v.cfg_predicates == predicates)?;
            if vm.raw_handle {
                return None;
            }
            let mut sig = method.sig.clone();
            resolve_signature_types(&mut sig, &method.attrs, &mut ctx);
            Some(quote! { (#sig) })
        })
        .collect();

    // Serialize default helper mappings for the proc macro
    let default_helper_tokens: Vec<proc_macro2::TokenStream> = default_helper_names
        .iter()
        .map(|(method_name_str, helper_ident)| {
            let method_ident = format_ident!("{method_name_str}");
            quote! { #method_ident => ($crate::#helper_ident) }
        })
        .collect();

    let trait_path_tokens = quote! { $crate::#trait_name };

    let local_type_aliases = ctx.local_type_aliases();

    // Generate &mut dyn Trait dispatch delegation (implies #[ffier::dispatch])
    // For traits with supertraits, we also need to delegate the supertrait methods.
    // Skip for foreign traits (orphan rules: can't impl foreign trait for local type).
    let dyn_dispatch = !is_foreign
        && !trait_item.supertraits.iter().any(|bound| thread_safety_bound(bound).is_none())
        && !vtable_methods.iter().any(|m| m.params.iter().any(ParamInfo::is_impl_trait))
        && trait_item.items.iter().all(|item| match item {
            TraitItem::Fn(method) => vtable_methods.iter().any(|vm| vm.name == method.sig.ident)
                && matches!(method.sig.inputs.first(), Some(FnArg::Receiver(r)) if r.reference.is_some())
                && method.sig.generics.type_params().next().is_none(),
            _ => false,
        });
    let boxdyn_impl = if dyn_dispatch {
        emit_dyn_dispatch_impl(&trait_item)
    } else {
        quote! {}
    };

    // For foreign traits (`extern trait`), don't re-emit the trait definition —
    // it already exists in the foreign crate. Only emit the generated plumbing.
    let trait_tokens = if is_foreign {
        quote! {}
    } else {
        quote! { #modified_trait }
    };
    let wrapper_thread_impls = thread_safety
        .rust_bounds()
        .map(|bound| {
            let bound: syn::Path = syn::parse_str(bound).expect("standard marker trait path");
            quote! {
                // SAFETY: unsafe construction requires the trait's callback contract.
                // Every safe Rust callback implementation satisfies its supertraits.
                unsafe impl #bound for #wrapper_name<'_> {}
            }
        })
        .collect::<Vec<_>>();
    let wrapper_thread_contract = format!(
        "Callback data and vtable must uphold the trait's thread-safety requirements: Send = {send}, Sync = {sync}. Send permits ownership transfer and destruction on another thread; Sync permits concurrent shared access only."
    );

    // When no_vtable is set, the @on_library_export arm emits nothing —
    // no vtable struct, no wrapper type, no FfiHandle/FfiType impls.
    // C callers cannot implement this trait; only concrete Rust types dispatch.
    let on_library_export_body = if no_vtable {
        quote! {}
    } else {
        quote! {
            // Vtable struct + trait impl generated by proc macro with
            // handle-type awareness for correct Result conventions.
            //
            // The proc macro output uses bare names (FfiType, FfiHandle, etc.)
            // which are brought into scope by `use $crate::*` inside const _.
            // The vtable struct is emitted OUTSIDE the const block so clients
            // can construct it. Its field types use fully-qualified paths
            // embedded from the bridge_type tokens in the metadata.
            ffier::__generate_vtable! {
                vtable_struct = #vtable_struct_name;
                wrapper = #wrapper_name;
                trait_path = (#trait_path_tokens);
                trait_generics = (#trait_ty_generics);
                crate_path = ($crate);
                handles = [$($__handle),*];
                values = [$($__value),*];
                num_slots = #num_slots;
                own_method_count = #own_method_count;
                methods = [#(#vtable_method_meta),*];
                method_sigs = [#(#vtable_method_sigs),*];
                default_helpers = [#(#default_helper_tokens),*];
            }

            #[repr(C)]
            pub struct #wrapper_name<'callback> {
                value: ffier::VtableHandle,
                lifetime: core::marker::PhantomData<&'callback mut ()>,
            }

            impl<'callback> #wrapper_name<'callback> {
                /// # Safety
                /// Callback data and vtable must remain valid for 'callback.
                /// This wrapper owns the callback and destroys it exactly once.
                #[doc = #wrapper_thread_contract]
                pub unsafe fn from_value(value: ffier::VtableHandle) -> Self {
                    Self { value, lifetime: core::marker::PhantomData }
                }
            }

            #(#wrapper_thread_impls)*

            impl Drop for #wrapper_name<'_> {
                fn drop(&mut self) {
                    let drop_field: Option<unsafe extern "C" fn(*mut core::ffi::c_void)> = unsafe {
                        self.value.field_or_none(0)
                    };
                    if let Some(drop_fn) = drop_field {
                        unsafe { drop_fn(self.value.user_data as *mut core::ffi::c_void) };
                    }
                }
            }

            impl FfiHandle for #wrapper_name<'_> {
                const C_HANDLE_NAME: &'static str = #wrapper_c_handle_suffix;
                const TYPE_TAG: u32 = $type_tag;
            }

            impl FfiType for #wrapper_name<'_> {
                type CRepr = *mut core::ffi::c_void;
                const C_TYPE_NAME: &'static str = #wrapper_c_handle_suffix;
                const IS_HANDLE: bool = true;
                fn into_c(self) -> *mut core::ffi::c_void {
                    ffier::ffier_handle_new(
                        <Self as FfiHandle>::TYPE_TAG,
                        self,
                    )
                }
                unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {
                    unsafe { ffier::ffier_handle_consume::<Self>(repr) }
                }
            }
        }
    };

    let cfg_pred_tokens = export_cfg.meta_tokens();
    let cfg_attr = export_cfg.cfg_attr();

    let mut reexport_names: Vec<_> = default_helper_names.values().collect();
    reexport_names.sort_by_key(|name| name.to_string());
    reexport_names.insert(0, &helper_mod_name);
    let (reexport_helpers, reexport_bundle) =
        reexport_helpers_with(&reexport_names, &cfg_attr, &default_helper_cfgs);

    let output = quote! {
        #reexport_bundle

        #cfg_attr
        #(#default_helpers)*

        #cfg_attr
        #trait_tokens

        #cfg_attr
        #thread_safety_check

        #cfg_attr
        #boxdyn_impl

        #cfg_attr
        #[doc(hidden)]
        pub mod #helper_mod_name {
            #(#local_type_aliases)*
        }

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            #reexport_helpers
            // @on_library_export: generates the vtable struct, wrapper type, its trait impl,
            // Drop, FfiHandle, and FfiType impls. Called by library_definition!
            // with the type tag.
            //
            // The vtable struct and wrapper type are emitted at the crate root of the
            // invoking crate, so orphan rules are satisfied even when the trait is
            // defined in an upstream crate.
            (@on_library_export, $type_tag:expr, [$($__handle:ident),*], [$($__value:ident),*]) => {
                #on_library_export_body
            };
            // Tagged invocation with path overrides (from library_definition!
            // shim for trait entries): the shim passes wrapper_name,
            // vtable_struct, and trait_path as parenthesized groups so the
            // metadata blob uses the library crate's paths for cross-crate
            // traits. Downstream crates in the chain only need the library
             // crate, not the upstream crate that defined the trait.
            ($prefix:literal, $type_tag:expr,
             ($($wrapper:tt)*), ($($tpath:tt)*),
             $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exported_trait,
                    trait_name = #trait_name,
                    trait_path = ($($tpath)*),
                    prefix = $prefix,
                    type_tag = $type_tag,
                    wrapper_name = ($($wrapper)*),
                    trait_lifetimes = (#(#trait_lifetime_idents),*),
                    vtable_methods = [#(#vtable_method_meta),*],
                    own_method_count = #own_method_count,
                    num_slots = #num_slots,
                    bless = #bless_tokens,
                    no_vtable = #no_vtable_tokens,
                    send = #send,
                    sync = #sync,
                    #cfg_pred_tokens
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
    };

    output.into()
}
