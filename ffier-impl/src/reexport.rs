//! Reexport FFI metadata and definition-site helper modules.

use std::collections::HashMap;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{ItemUse, parse_macro_input};

/// Forward definition-site helpers when an annotated item is re-exported.
/// Metadata uses `$crate` paths, so a submodule's helpers must be visible at
/// its defining crate's root before another crate registers that item.
pub(crate) fn reexport_helpers(
    names: &[&syn::Ident],
    cfg_attr: &proc_macro2::TokenStream,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    reexport_helpers_with(names, cfg_attr, &HashMap::new())
}

/// [`reexport_helpers`] with per-name predicates.
///
/// Helpers generated for cfg-gated methods only exist in the configurations
/// where their method exists, so their reexports must carry the same predicate.
/// One helper ident may be shared by several cfg-exclusive variants of a method,
/// so every predicate that makes it exist gets its own reexport.
pub(crate) fn reexport_helpers_with(
    names: &[&syn::Ident],
    cfg_attr: &proc_macro2::TokenStream,
    name_cfgs: &HashMap<String, Vec<proc_macro2::TokenStream>>,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let helper_mod = names[0];
    let bundle_name = format_ident!("{helper_mod}_reexports");
    let arm = quote! {
        (@reexport_helpers ($($parent:tt)*)) => {
            #[doc(hidden)]
            #[allow(unused_imports)]
            pub use $($parent)* #bundle_name::*;
        };
    };
    let mut reexports: Vec<proc_macro2::TokenStream> = Vec::new();
    for name in names {
        match name_cfgs.get(&name.to_string()) {
            Some(preds) => reexports.extend(preds.iter().map(|pred| {
                // The helper exists only where both the trait and its method do.
                let guard = quote! { #cfg_attr #[cfg(#pred)] };
                quote! { #guard pub use super::#name; }
            })),
            None => reexports.push(quote! { #cfg_attr pub use super::#name; }),
        }
    }
    // A glob import can coexist with the explicit helper imports emitted by
    // qualified library_definition! entries, without duplicate-name errors.
    let bundle = quote! {
        #[doc(hidden)]
        pub mod #bundle_name {
            pub use super::#bundle_name;
            // Evaluate the provider's cfg here, never in the importing crate.
            #(#reexports)*
        }
    };
    (arm, bundle)
}

pub(crate) fn reexport(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let use_item = parse_macro_input!(item as ItemUse);

    let mut meta_reexports = Vec::new();
    collect_reexport_paths(&use_item.tree, &mut Vec::new(), &mut meta_reexports);

    let output = quote! {
        #use_item

        #(
            #[doc(hidden)]
            #meta_reexports
        )*
    };

    output.into()
}

/// Walk a `UseTree` and collect `pub use path::__ffier_meta_Type;` statements
/// for each simple (non-glob, non-renamed) leaf.
fn collect_reexport_paths(
    tree: &syn::UseTree,
    prefix: &mut Vec<syn::Ident>,
    out: &mut Vec<proc_macro2::TokenStream>,
) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.clone());
            collect_reexport_paths(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let type_name = &n.ident;
            let meta_alias = format_ident!("__ffier_meta_{type_name}");
            if prefix.is_empty() {
                out.push(quote! { pub use #meta_alias; });
            } else {
                out.push(quote! {
                    pub use #(#prefix)::*::#meta_alias;
                    #(#prefix)::*::#meta_alias!(@reexport_helpers (#(#prefix)::*::));
                });
            }
        }
        syn::UseTree::Group(g) => {
            for tree in &g.items {
                collect_reexport_paths(tree, prefix, out);
            }
        }
        syn::UseTree::Rename(_) => {
            // Renamed imports are not supported — the user can manually
            // re-export the metadata if needed.
        }
        syn::UseTree::Glob(_) => {
            // Glob imports can't be handled — we don't know which types exist.
        }
    }
}
