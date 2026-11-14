//! Export standalone functions.

use std::sync::atomic::Ordering;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::FnArg;

use super::ExportCfg;
use crate::{
    MACRO_COUNTER,
    alias::AliasContext,
    attrs::{parse_ffier_method_attrs, strip_ffier_attrs},
    reexport::reexport_helpers,
    signature::{MethodMetaKind, emit_method_meta, parse_method_sig},
};

/// Handle `#[ffier::export]` on a free (non-method) function.
pub(super) fn exportable_free_fn(input: syn::ItemFn, export_cfg: &ExportCfg) -> TokenStream {
    let fn_name = &input.sig.ident;
    let fn_name_str = fn_name.to_string();

    let helper_mod_name = format_ident!("_ffier_fn_{fn_name_str}");
    let mut ctx = AliasContext::new(helper_mod_name.clone());

    // Parse the function signature as a static method (no self)
    let method_ffier = match parse_ffier_method_attrs(&input.attrs) {
        Ok(attrs) => attrs,
        Err(error) => return error.to_compile_error().into(),
    };
    if method_ffier.foreign_return.is_some() && method_ffier.foreign_return_c_name.is_none() {
        return syn::Error::new_spanned(
            &input.sig.ident,
            "#[ffier(foreign_return = ...)] requires c_name = \"...\" \
             (the C typedef name for the foreign handle type)",
        )
        .to_compile_error()
        .into();
    }
    let foreign_ret = method_ffier.foreign_return.as_ref();
    let foreign_ret_c = method_ffier.foreign_return_c_name.as_deref();
    let mut method = match parse_method_sig(
        &input.sig,
        &input.attrs,
        &mut ctx,
        None,
        false,
        false,
        foreign_ret,
        foreign_ret_c,
    ) {
        Ok(Some(m)) => m,
        Err(e) => return e.to_compile_error().into(),
        Ok(None) => {
            return syn::Error::new_spanned(
                &input.sig,
                "ffier: could not parse free function signature",
            )
            .to_compile_error()
            .into();
        }
    };
    method.ffi_name = fn_name_str.clone();

    let local_type_aliases = ctx.local_type_aliases();
    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_fn_{fn_name_str}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{fn_name}");

    let method_meta_tokens = emit_method_meta(&[method], MethodMetaKind::Impl);
    let fn_path = quote! { $crate::#fn_name };

    // Strip #[ffier(...)] attributes from the function and its params
    let clean_fn = {
        let mut f = input.clone();
        strip_ffier_attrs(&mut f.attrs);
        for arg in &mut f.sig.inputs {
            if let FnArg::Typed(pat_ty) = arg {
                strip_ffier_attrs(&mut pat_ty.attrs);
            }
        }
        f
    };

    let cfg_pred_tokens = export_cfg.meta_tokens();
    let cfg_attr = export_cfg.cfg_attr();

    let (reexport_helpers, reexport_bundle) = reexport_helpers(&[&helper_mod_name], &cfg_attr);

    let output = quote! {
        #reexport_bundle

        #cfg_attr
        #clean_fn

        #cfg_attr
        #[doc(hidden)]
        pub mod #helper_mod_name {
            #(#local_type_aliases)*
        }

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            #reexport_helpers
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exported_fn,
                    name = #fn_name,
                    fn_path = (#fn_path),
                    prefix = $prefix,
                    ffi_name = #fn_name_str,
                    methods = [#(#method_meta_tokens),*],
                    #cfg_pred_tokens
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
    };

    output.into()
}
