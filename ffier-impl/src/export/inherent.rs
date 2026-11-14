//! Export inherent implementations of opaque handle types.

use std::sync::atomic::Ordering;

use ffier_schema::ThreadSafety;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ImplItem, ItemImpl, ReturnType, Type};

use super::ExportCfg;
use crate::{
    MACRO_COUNTER,
    alias::AliasContext,
    attrs::{parse_ffier_method_attrs, strip_ffier_attrs},
    contracts::{lifetime_variance_assertions, thread_safety_assertion, validate_handle_variance},
    meta::{camel_to_snake, erase_lifetimes},
    reexport::reexport_helpers,
    signature::{MethodMetaKind, emit_method_meta, parse_method_sig},
    types::check_forbidden_ffi_type,
};

pub(super) fn exportable_struct_impl(input: ItemImpl, export_cfg: &ExportCfg) -> TokenStream {
    // Reject type/const generics, bounded lifetimes, and specialization now:
    // every later helper assumes unconstrained lifetime parameters only.
    let variances = match validate_handle_variance(&input, export_cfg) {
        Ok(variances) => variances,
        Err(error) => return error.to_compile_error().into(),
    };

    // Strip #[ffier(...)] attributes from methods before emitting the impl block
    let impl_block = {
        let mut block = input.clone();
        for item in &mut block.items {
            if let ImplItem::Fn(method) = item {
                strip_ffier_attrs(&mut method.attrs);
                for arg in &mut method.sig.inputs {
                    if let FnArg::Typed(pat_ty) = arg {
                        strip_ffier_attrs(&mut pat_ty.attrs);
                    }
                }
            }
        }
        block
    };

    let Type::Path(ref struct_path) = *input.self_ty else {
        return syn::Error::new_spanned(&input.self_ty, "expected a named struct type")
            .to_compile_error()
            .into();
    };
    let last_seg = struct_path
        .path
        .segments
        .last()
        .expect("expected struct name");
    let struct_ident = &last_seg.ident;
    let self_ty = &input.self_ty;
    let struct_name = struct_ident.to_string();
    let struct_lower = camel_to_snake(&struct_name);

    let helper_mod_name = format_ident!("_ffier_{struct_lower}");
    let mut ctx = AliasContext::new(helper_mod_name.clone());

    let mut methods = Vec::new();
    let is_inherent = input.trait_.is_none();
    let mut warnings = Vec::new();

    for item in &input.items {
        let ImplItem::Fn(method) = item else { continue };

        // Check for forbidden FFI types in params and return
        for arg in &method.sig.inputs {
            if let FnArg::Typed(pat_ty) = arg
                && let Some(msg) = check_forbidden_ffi_type(&pat_ty.ty)
            {
                return syn::Error::new_spanned(&pat_ty.ty, msg)
                    .to_compile_error()
                    .into();
            }
        }
        if let ReturnType::Type(_, ty) = &method.sig.output
            && let Some(msg) = check_forbidden_ffi_type(ty)
        {
            return syn::Error::new_spanned(ty, msg).to_compile_error().into();
        }

        // Skip non-public methods in inherent impls (bridge crate can't call them)
        if is_inherent && !matches!(method.vis, syn::Visibility::Public(_)) {
            let msg = format!(
                "ffier: skipping non-public method `{}`; make it `pub` to export via FFI",
                method.sig.ident
            );
            warnings.push(quote::quote_spanned! { method.sig.ident.span() =>
                const _: () = {
                    #[deprecated = #msg]
                    const WARNING: () = ();
                    let _ = WARNING;
                };
            });
            continue;
        }

        // Parse method-level #[ffier(...)] for foreign_return
        let method_ffier = match parse_ffier_method_attrs(&method.attrs) {
            Ok(attrs) => attrs,
            Err(error) => return error.to_compile_error().into(),
        };
        if method_ffier.foreign_return.is_some() && method_ffier.foreign_return_c_name.is_none() {
            return syn::Error::new_spanned(
                &method.sig.ident,
                "#[ffier(foreign_return = ...)] requires c_name = \"...\" \
                 (the C typedef name for the foreign handle type)",
            )
            .to_compile_error()
            .into();
        }
        let foreign_ret = method_ffier.foreign_return.as_ref();
        let foreign_ret_c = method_ffier.foreign_return_c_name.as_deref();
        match parse_method_sig(
            &method.sig,
            &method.attrs,
            &mut ctx,
            Some(self_ty),
            false,
            false,
            foreign_ret,
            foreign_ret_c,
        ) {
            Ok(Some(mut m)) => {
                m.ffi_name = format!("{}_{}", struct_lower, method.sig.ident);
                methods.push(m);
            }
            Ok(None) => {}
            Err(e) => return e.to_compile_error().into(),
        }
    }

    // -----------------------------------------------------------------------
    // Metadata emission — structured tokens for generator proc macros
    // -----------------------------------------------------------------------

    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{struct_lower}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{struct_ident}");
    let native_alias_name = format_ident!("__FfierNativeType{counter}");
    let (impl_generics, type_generics, _) = input.generics.split_for_impl();

    // Resolve the exact checked type beside the impl, including relative paths.
    // Registration and external bridges must never resolve its bare name again.
    // A qualified impl can verify `other::Foo` while `$crate::Foo` names a
    // different native type; Send/Sync and future capability proofs must be
    // attached to the type the impl actually checked, not a same-named root type.
    let native_type = quote! { $crate::#helper_mod_name::__NativeType #type_generics };
    let native_bridge_type = quote! { super::#native_alias_name };
    ctx.current_cfg = None;
    let struct_path_tokens = ctx.alias_tokens(&erase_lifetimes(self_ty));
    let local_type_aliases = ctx.local_type_aliases_for_native(self_ty, &native_bridge_type);

    let method_meta_tokens = emit_method_meta(&methods, MethodMetaKind::Impl);

    // Lifetime idents (without the tick) for metadata
    let lifetime_idents: Vec<_> = input
        .generics
        .lifetimes()
        .map(|lt| format_ident!("{}", lt.lifetime.ident))
        .collect();

    let lifetime_meta: Vec<_> = lifetime_idents
        .iter()
        .zip(&variances)
        .map(|(name, variance)| {
            let variance = format_ident!("{}", variance.as_str());
            quote! { (#name, #variance) }
        })
        .collect();
    let variance_checks = lifetime_variance_assertions(&input, &variances);

    let cfg_pred_tokens = export_cfg.meta_tokens();
    let cfg_attr = export_cfg.cfg_attr();
    let ThreadSafety { send, sync } = export_cfg.thread_safety;
    let thread_safety_check =
        thread_safety_assertion(self_ty, &input.generics, export_cfg.thread_safety);

    let (reexport_helpers, reexport_bundle) = reexport_helpers(&[&helper_mod_name], &cfg_attr);

    let output = quote! {
        #reexport_bundle

        #cfg_attr
        #impl_block

        #cfg_attr
        #thread_safety_check

        #cfg_attr
        #variance_checks

        #(#warnings)*

        #cfg_attr
        #[doc(hidden)]
        #[allow(private_interfaces)]
        pub type #native_alias_name #impl_generics = #self_ty;

        #cfg_attr
        #[doc(hidden)]
        pub mod #helper_mod_name {
            pub use super::#native_alias_name as __NativeType;
            #(#local_type_aliases)*
        }

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            #reexport_helpers
            (@on_library_export, $type_tag:expr, [$($__handle:ident),*], [$($__value:ident),*]) => {
                impl #impl_generics FfiHandle for #native_type {
                    const C_HANDLE_NAME: &'static str = stringify!(#struct_ident);
                    const TYPE_TAG: u32 = $type_tag;
                }
                impl #impl_generics FfiType for #native_type {
                    type CRepr = *mut core::ffi::c_void;
                    const C_TYPE_NAME: &'static str = stringify!(#struct_ident);
                    const IS_HANDLE: bool = true;
                    fn into_c(self) -> *mut core::ffi::c_void {
                        ffier::ffier_handle_new($type_tag, self)
                    }
                    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {
                        unsafe { ffier::ffier_handle_consume::<Self>(repr) }
                    }
                }
            };
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exported_impl,
                    name = #struct_ident,
                    struct_path = (#struct_path_tokens),
                    prefix = $prefix,
                    type_tag = $type_tag,
                    lifetimes = (#(#lifetime_meta),*),
                    send = #send,
                    sync = #sync,
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
