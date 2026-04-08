//! Client code generation from parsed metadata.
//!
//! `generate` takes batched metadata token streams and produces safe Rust
//! wrapper code that calls through C ABI extern declarations.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use std::collections::HashSet;

use ffier_meta::{
    MetaError, MetaExportable, MetaImplementable, MetaParamKind, MetaReceiver, MetaReturn,
    MetaTraitImpl, MetaValueKind, MetaVtableRet, camel_to_snake, peek_meta_tag,
};

/// Generates Rust client source code from batched metadata.
///
/// Receives `{ @tag, ... } { @tag, ... } ...` — multiple metadata items.
/// Sorts by category, generates all client code, and emits it as a single
/// `const FFIER_ALL_CLIENT_SRC: &str = "..."`.
#[proc_macro]
pub fn generate(input: TokenStream) -> TokenStream {
    generate_batch_client_impl(input.into()).into()
}

fn generate_batch_client_impl(input: TokenStream2) -> TokenStream2 {
    let mut errors = Vec::new();
    let mut exportables = Vec::new();
    let mut implementables = Vec::new();
    let mut trait_impls = Vec::new();

    for tt in input {
        if let proc_macro2::TokenTree::Group(g) = tt {
            if g.delimiter() == proc_macro2::Delimiter::Brace {
                let stream = g.stream();
                match peek_meta_tag(&stream).as_str() {
                    "error" => errors.push(stream),
                    "exportable" => exportables.push(stream),
                    "implementable" => implementables.push(stream),
                    "trait_impl" => trait_impls.push(stream),
                    tag => {
                        let msg = format!("unknown metadata tag `@{tag}` in batch");
                        return quote! { compile_error!(#msg); };
                    }
                }
            }
        }
    }

    // Track which traits have been defined to avoid duplicate definitions.
    let mut defined_traits = HashSet::new();
    let mut all_source = String::new();

    // Process in sorted order: errors → exportables → implementables → trait_impls.
    for item in errors.iter().chain(exportables.iter()) {
        let code = generate_one(item.clone());
        all_source.push_str(&code.to_string());
        all_source.push('\n');
    }

    for item in &implementables {
        let meta: MetaImplementable = match syn::parse2(item.clone()) {
            Ok(m) => m,
            Err(e) => return e.to_compile_error(),
        };
        defined_traits.insert(meta.trait_name.to_string());
        let code = generate_implementable_client(meta);
        all_source.push_str(&code.to_string());
        all_source.push('\n');
    }

    for item in &trait_impls {
        let meta: MetaTraitImpl = match syn::parse2(item.clone()) {
            Ok(m) => m,
            Err(e) => return e.to_compile_error(),
        };
        let emit_trait_def = defined_traits.insert(meta.trait_name.to_string());
        let code = generate_trait_impl_client(meta, emit_trait_def);
        all_source.push_str(&code.to_string());
        all_source.push('\n');
    }

    quote! { const FFIER_ALL_CLIENT_SRC: &str = #all_source; }
}

fn generate_one(input: TokenStream2) -> TokenStream2 {
    let tag = peek_meta_tag(&input);
    match tag.as_str() {
        "exportable" => {
            let meta: MetaExportable = match syn::parse2(input) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_exportable_client(meta)
        }
        "error" => {
            let meta: MetaError = match syn::parse2(input) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_error_client(meta)
        }
        _ => {
            let msg = format!("unknown metadata tag `@{tag}`");
            quote! { compile_error!(#msg); }
        }
    }
}


// ===========================================================================
// Exportable client generation
// ===========================================================================

fn generate_exportable_client(meta: MetaExportable) -> TokenStream2 {
    let struct_name = &meta.struct_name;
    let struct_name_str = struct_name.to_string();
    let struct_lower = camel_to_snake(&struct_name_str);
    let type_pfx = meta.type_pfx();
    let fn_pfx = meta.fn_pfx();

    let has_lifetimes = !meta.lifetimes.is_empty();

    // Lifetime tokens prefixed with ' --- use mixed_site span so the lifetimes
    // resolve in the client crate context, not the library crate where the
    // metadata was originally defined.
    let client_struct_generics_with_tick = if has_lifetimes {
        let lts: Vec<_> = meta
            .lifetimes
            .iter()
            .map(|lt| {
                let lt_lifetime =
                    syn::Lifetime::new(&format!("'{lt}"), proc_macro2::Span::call_site());
                quote! { #lt_lifetime }
            })
            .collect();
        quote! { <#(#lts),*> }
    } else {
        quote! {}
    };

    let client_phantom = if has_lifetimes {
        let lts: Vec<_> = meta
            .lifetimes
            .iter()
            .map(|lt| {
                let lt_lifetime =
                    syn::Lifetime::new(&format!("'{lt}"), proc_macro2::Span::call_site());
                quote! { &#lt_lifetime () }
            })
            .collect();
        quote! { , std::marker::PhantomData<(#(#lts),*)> }
    } else {
        quote! {}
    };
    let client_phantom_init = if has_lifetimes {
        quote! { , std::marker::PhantomData }
    } else {
        quote! {}
    };

    // FfiHandle for all types (including lifetime-parameterized)
    let client_ffi_handle_impl = quote! {
        impl #client_struct_generics_with_tick ffier::FfiHandle for #struct_name #client_struct_generics_with_tick {
            const C_HANDLE_NAME: &'static str = #struct_name_str;
            fn as_handle(&self) -> *mut core::ffi::c_void { self.0 }
        }
    };

    // FfiType for all types (including lifetime-parameterized)
    let client_ffi_type_impl = quote! {
        impl #client_struct_generics_with_tick ffier::FfiType for #struct_name #client_struct_generics_with_tick {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &'static str = #struct_name_str;
            fn into_c(self) -> *mut core::ffi::c_void { self.__into_raw() }
            fn from_c(repr: *mut core::ffi::c_void) -> Self { Self::__from_raw(repr) }
        }
    };

    // Destroy function
    let destroy_name_str = format!("{}{}_destroy", fn_pfx, struct_lower);
    let destroy_ident = format_ident!("{destroy_name_str}");

    let mut client_extern_decls = Vec::new();
    let mut client_methods = Vec::new();

    // Destroy extern decl
    client_extern_decls.push(quote! {
        pub fn #destroy_ident(handle: *mut core::ffi::c_void);
    });

    for m in &meta.methods {
        let method_name = &m.name;

        let has_receiver = m.receiver != MetaReceiver::None;
        let is_mut = m.receiver == MetaReceiver::Mut;
        let is_by_value = m.receiver == MetaReceiver::Value;
        let is_builder = m.is_builder;
        let handle_is_indirect = is_builder && is_by_value;

        // --- Build extern "C" declaration from C generator's signature ---
        // for_client=true uses rust_type instead of bridge_type ($crate:: paths)
        let c_sig = ffier_gen_c::c_signature_for_method(
            m, &meta.prefix, ffier_gen_c::SignatureContext::Client,
        );
        let ffi_name = format_ident!("{}", c_sig.fn_name);
        let sig_param_names: Vec<_> = c_sig.params.iter().map(|p| &p.name).collect();
        let sig_param_types: Vec<_> = c_sig.params.iter().map(|p| &p.c_type).collect();
        let sig_ret = &c_sig.ret;

        client_extern_decls.push(quote! {
            pub fn #ffi_name(#(#sig_param_names: #sig_param_types),*) #sig_ret;
        });

        // --- Build safe wrapper method ---

        // Receiver
        let wrapper_receiver = if !has_receiver {
            None
        } else if is_by_value {
            Some(quote! { self, })
        } else if is_mut {
            Some(quote! { &mut self, })
        } else {
            Some(quote! { &self, })
        };

        // Wrapper params (original Rust types)
        let wrapper_params: Vec<_> = m
            .params
            .iter()
            .map(|p| {
                let id = &p.name;
                match &p.kind {
                    MetaParamKind::StrSlice => quote! { #id: &[&str] },
                    MetaParamKind::ImplTrait { .. } => {
                        let rust_type = p.rust_type.as_ref().unwrap();
                        quote! { #id: #rust_type }
                    }
                    _ => {
                        let rust_type = p
                            .rust_type
                            .as_ref()
                            .expect("param must have rust_type");
                        quote! { #id: #rust_type }
                    }
                }
            })
            .collect();

        // Arg conversions (Rust value -> FFI call arg)
        let wrapper_args: Vec<_> = m
            .params
            .iter()
            .map(|p| {
                let id = &p.name;
                match &p.kind {
                    MetaParamKind::StrSlice => {
                        quote! { __ffi_strs.as_ptr(), __ffi_strs.len() }
                    }
                    MetaParamKind::ImplTrait { .. } => {
                        quote! { #id.__into_raw_handle() }
                    }
                    MetaParamKind::Regular { .. } => {
                        let rust_type = p.rust_type.as_ref().unwrap();
                        quote! { <#rust_type as ffier::FfiType>::into_c(#id) }
                    }
                }
            })
            .collect();

        // Pre-bindings for StrSlice
        let wrapper_pre_bindings: Vec<_> = m
            .params
            .iter()
            .filter_map(|p| {
                let id = &p.name;
                if matches!(p.kind, MetaParamKind::StrSlice) {
                    Some(quote! {
                        let __ffi_strs: Vec<ffier::FfierBytes> = #id.iter()
                            .map(|s| unsafe { ffier::FfierBytes::from_str(s) })
                            .collect();
                    })
                } else {
                    None
                }
            })
            .collect();

        // Handle arg for calling FFI
        let handle_arg = if !has_receiver {
            quote! {}
        } else if handle_is_indirect {
            quote! { &mut __handle, }
        } else {
            quote! { self.0, }
        };

        // Build the method body
        let wrapper_body = if is_builder && is_by_value {
            // Builder pattern
            match &m.ret {
                MetaReturn::Void => {
                    quote! {
                        let mut __handle = {
                            let this = std::mem::ManuallyDrop::new(self);
                            this.0
                        };
                        #(#wrapper_pre_bindings)*
                        unsafe { #ffi_name(&mut __handle, #(#wrapper_args),*) };
                        Self(__handle #client_phantom_init)
                    }
                }
                MetaReturn::Result {
                    ok: None,
                    err_ident,
                    ..
                } => {
                    let err_ty = format_ident!("{err_ident}");
                    quote! {
                        let mut __handle = {
                            let this = std::mem::ManuallyDrop::new(self);
                            this.0
                        };
                        #(#wrapper_pre_bindings)*
                        let __err = unsafe { #ffi_name(&mut __handle, #(#wrapper_args),*) };
                        if __err.code == 0 {
                            Ok(Self(__handle #client_phantom_init))
                        } else {
                            Err(#err_ty::from_ffi(__err))
                        }
                    }
                }
                _ => quote! { compile_error!("unexpected builder return kind") },
            }
        } else if is_by_value && !has_receiver {
            // Static method
            build_client_body(
                &ffi_name,
                &m.ret,
                &handle_arg,
                &wrapper_args,
                &wrapper_pre_bindings,
                &m.rust_ret,
                &type_pfx,
            )
        } else if is_by_value {
            // By-value self, non-builder
            let inner_body = build_client_body(
                &ffi_name,
                &m.ret,
                &quote! { __handle, },
                &wrapper_args,
                &wrapper_pre_bindings,
                &m.rust_ret,
                &type_pfx,
            );
            quote! {
                let __handle = {
                    let this = std::mem::ManuallyDrop::new(self);
                    this.0
                };
                #inner_body
            }
        } else {
            // &self or &mut self
            build_client_body(
                &ffi_name,
                &m.ret,
                &handle_arg,
                &wrapper_args,
                &wrapper_pre_bindings,
                &m.rust_ret,
                &type_pfx,
            )
        };

        // Doc comments
        let doc_attrs: Vec<_> = m.doc.iter().map(|line| quote! { #[doc = #line] }).collect();

        // Return type for safe wrapper signature
        let rust_ret = &m.rust_ret;
        let wrapper_ret_type = match &m.ret {
            MetaReturn::Void if is_builder => quote! { -> Self },
            MetaReturn::Void => quote! {},
            MetaReturn::Value(_) => {
                if rust_ret.is_empty() {
                    quote! {}
                } else {
                    quote! { -> #rust_ret }
                }
            }
            MetaReturn::Result { err_ident, .. } if is_builder => {
                let err_ty = format_ident!("{err_ident}");
                quote! { -> Result<Self, #err_ty> }
            }
            MetaReturn::Result { ok, err_ident, .. } => {
                let err_ty = format_ident!("{err_ident}");
                match ok {
                    None => quote! { -> Result<(), #err_ty> },
                    Some(_) => {
                        // Use rust_ret which contains the full Result<T, E> type
                        quote! { -> #rust_ret }
                    }
                }
            }
        };

        // Method-level lifetime generics (e.g. <'a, 'b>)
        let method_generics = if m.method_lifetimes.is_empty() {
            quote! {}
        } else {
            let lts: Vec<_> = m
                .method_lifetimes
                .iter()
                .map(|lt| syn::Lifetime::new(&format!("'{lt}"), proc_macro2::Span::call_site()))
                .collect();
            quote! { <#(#lts),*> }
        };

        client_methods.push(quote! {
            #(#doc_attrs)*
            pub fn #method_name #method_generics(#wrapper_receiver #(#wrapper_params),*) #wrapper_ret_type {
                #wrapper_body
            }
        });
    }

    quote! {
        unsafe extern "C" {
            #(#client_extern_decls)*
        }

        pub struct #struct_name #client_struct_generics_with_tick (
            *mut core::ffi::c_void
            #client_phantom
        );

        impl #client_struct_generics_with_tick #struct_name #client_struct_generics_with_tick {
            #[doc(hidden)]
            pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
                Self(ptr #client_phantom_init)
            }

            #[doc(hidden)]
            pub fn __into_raw(self) -> *mut core::ffi::c_void {
                let this = std::mem::ManuallyDrop::new(self);
                this.0
            }
        }

        #client_ffi_handle_impl
        #client_ffi_type_impl

        impl #client_struct_generics_with_tick std::fmt::Debug for #struct_name #client_struct_generics_with_tick {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_tuple(#struct_name_str).field(&self.0).finish()
            }
        }

        impl #client_struct_generics_with_tick #struct_name #client_struct_generics_with_tick {
            #(#client_methods)*
        }

        impl #client_struct_generics_with_tick Drop for #struct_name #client_struct_generics_with_tick {
            fn drop(&mut self) {
                unsafe { #destroy_ident(self.0) }
            }
        }

    }
}

// ===========================================================================
// Error client generation
// ===========================================================================

fn generate_error_client(meta: MetaError) -> TokenStream2 {
    let name = &meta.name;
    let name_str = name.to_string();

    let variant_names: Vec<_> = meta.variants.iter().map(|v| &v.name).collect();

    let match_arms_from_ffi: Vec<_> = meta
        .variants
        .iter()
        .map(|v| {
            let vname = &v.name;
            let code = v.code;
            quote! { #code => Self::#vname, }
        })
        .collect();

    let match_arms_display: Vec<_> = meta
        .variants
        .iter()
        .map(|v| {
            let vname = &v.name;
            let msg = &v.message;
            quote! { Self::#vname => write!(f, #msg), }
        })
        .collect();

    quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum #name {
            #(#variant_names),*
        }

        impl #name {
            pub fn from_ffi(mut err: ffier::FfierError) -> Self {
                let code = err.code;
                unsafe { err.free() };
                match code {
                    #(#match_arms_from_ffi)*
                    other => panic!("unknown {} error code {}", #name_str, other),
                }
            }
        }

        impl std::fmt::Display for #name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    #(#match_arms_display)*
                }
            }
        }

        impl std::error::Error for #name {}
    }
}

// ===========================================================================
// Implementable client generation
// ===========================================================================

fn generate_implementable_client(meta: MetaImplementable) -> TokenStream2 {
    // Use plain ident names for client types (not $crate:: paths from bridge)
    let vtable_struct_name = format_ident!(
        "{}",
        meta.vtable_struct_name
            .to_string()
            .split("::")
            .last()
            .unwrap_or("VtableStruct")
            .trim()
    );
    let wrapper_name = format_ident!(
        "{}",
        meta.wrapper_name
            .to_string()
            .split("::")
            .last()
            .unwrap_or("VtableWrapper")
            .trim()
    );
    let constructor_name = format_ident!("{}", meta.constructor_name());

    // Vtable fields (user-defined fields)
    let vtable_field_defs: Vec<_> = meta
        .vtable_fields
        .iter()
        .map(|f| {
            let fname = &f.name;
            let ftype = &f.field_type;
            quote! { pub #fname: #ftype, }
        })
        .collect();

    // Vtable method function pointer fields
    let vtable_method_fields: Vec<_> = meta
        .vtable_methods
        .iter()
        .map(|m| {
            let mname = &m.name;
            let params: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let bt = &p.bridge_type;
                    quote! { <#bt as ffier::FfiType>::CRepr }
                })
                .collect();
            let ret = match &m.ret {
                MetaVtableRet::Void => quote! {},
                MetaVtableRet::Value { bridge_type, .. } => {
                    quote! { -> <#bridge_type as ffier::FfiType>::CRepr }
                }
            };
            quote! {
                pub #mname: unsafe extern "C" fn(
                    *mut core::ffi::c_void
                    #(, #params)*
                ) #ret,
            }
        })
        .collect();

    let trait_name = &meta.trait_name;
    let trait_method_sigs: Vec<_> = meta
        .vtable_methods
        .iter()
        .map(|m| {
            let mname = &m.name;
            let params: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let id = &p.name;
                    let rt = &p.rust_type;
                    quote! { #id: #rt }
                })
                .collect();
            let ret = match &m.ret {
                MetaVtableRet::Void => quote! {},
                MetaVtableRet::Value { rust_type, .. } => quote! { -> #rust_type },
            };
            quote! { fn #mname(&self, #(#params),*) #ret; }
        })
        .collect();

    // Build vtable field initializers with const-promoted trampolines for the default method
    let vtable_trampoline_fields: Vec<_> = meta
        .vtable_methods
        .iter()
        .map(|m| {
            let mname = &m.name;
            // Trampoline fn signature uses bridge_type ('static lifetimes)
            // for the CRepr types, since it's a standalone extern fn.
            // Conversions (from_c/into_c) use rust_type (actual lifetimes)
            // so the trait method's lifetimes are preserved.
            let params: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let id = &p.name;
                    let bt = &p.bridge_type;
                    quote! { #id: <#bt as ffier::FfiType>::CRepr }
                })
                .collect();
            let ret = match &m.ret {
                MetaVtableRet::Void => quote! {},
                MetaVtableRet::Value { bridge_type, .. } => {
                    quote! { -> <#bridge_type as ffier::FfiType>::CRepr }
                }
            };

            // Argument conversions use rust_type (actual lifetimes)
            let arg_conversions: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let id = &p.name;
                    let rt = &p.rust_type;
                    quote! { <#rt as ffier::FfiType>::from_c(#id) }
                })
                .collect();

            // Return conversion uses rust_type (actual lifetimes)
            let ret_conversion = match &m.ret {
                MetaVtableRet::Void => quote! { __result },
                MetaVtableRet::Value { rust_type, .. } => {
                    quote! { <#rust_type as ffier::FfiType>::into_c(__result) }
                }
            };

            quote! {
                #mname: {
                    unsafe extern "C" fn __trampoline<__T: #trait_name>(
                        __ud: *mut core::ffi::c_void
                        #(, #params)*
                    ) #ret {
                        let __obj = unsafe { &*(__ud as *const __T) };
                        let __result = __obj.#mname(#(#arg_conversions),*);
                        #ret_conversion
                    }
                    __trampoline::<Self>
                }
            }
        })
        .collect();

    quote! {
        pub trait #trait_name {
            #(#trait_method_sigs)*

            /// Convert this value into an opaque FFI handle via vtable dispatch.
            ///
            /// Known types (with `#[ffier::trait_impl]`) override this with
            /// direct handle passthrough. User types get the default
            /// implementation which builds a const-promoted static vtable.
            #[doc(hidden)]
            fn __into_raw_handle(self) -> *mut core::ffi::c_void where Self: Sized {
                let __vtable: &'static #vtable_struct_name = &#vtable_struct_name {
                    #(#vtable_trampoline_fields,)*
                    drop: Some({
                        unsafe extern "C" fn __trampoline<__T>(
                            __ud: *mut core::ffi::c_void,
                        ) {
                            unsafe { drop(Box::from_raw(__ud as *mut __T)) };
                        }
                        __trampoline::<Self>
                    }),
                };
                let __ud = Box::into_raw(Box::new(self)) as *mut core::ffi::c_void;
                #wrapper_name::new(__ud, __vtable).__into_raw()
            }
        }

        #[repr(C)]
        pub struct #vtable_struct_name {
            #(#vtable_field_defs)*
            #(#vtable_method_fields)*
            pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
        }

        unsafe extern "C" {
            pub fn #constructor_name(
                user_data: *mut core::ffi::c_void,
                vtable: *const #vtable_struct_name,
            ) -> *mut core::ffi::c_void;
        }

        pub struct #wrapper_name(*mut core::ffi::c_void);

        impl #wrapper_name {
            pub fn new(user_data: *mut core::ffi::c_void, vtable: &#vtable_struct_name) -> Self {
                Self(unsafe { #constructor_name(user_data, vtable) })
            }

            #[doc(hidden)]
            pub fn __into_raw(self) -> *mut core::ffi::c_void {
                let this = std::mem::ManuallyDrop::new(self);
                this.0
            }
        }

        impl Drop for #wrapper_name {
            fn drop(&mut self) {
                // vtable handles are consumed by dyn_param methods, no separate destroy
            }
        }
    }
}

// ===========================================================================
// Client codegen helpers
// ===========================================================================

/// Build the method body for a non-builder static or instance method.
fn build_client_body(
    ffi_name: &syn::Ident,
    ret: &MetaReturn,
    handle_arg: &TokenStream2,
    wrapper_args: &[TokenStream2],
    wrapper_pre_bindings: &[TokenStream2],
    rust_ret: &TokenStream2,
    _type_pfx: &str,
) -> TokenStream2 {
    match ret {
        MetaReturn::Void => {
            quote! {
                #(#wrapper_pre_bindings)*
                unsafe { #ffi_name(#handle_arg #(#wrapper_args),*) }
            }
        }
        MetaReturn::Value(vk) => {
            let convert = client_value_from_ffi(vk, rust_ret);
            quote! {
                #(#wrapper_pre_bindings)*
                let __raw = unsafe { #ffi_name(#handle_arg #(#wrapper_args),*) };
                #convert
            }
        }
        MetaReturn::Result { ok, err_ident, .. } => {
            let err_ty = format_ident!("{err_ident}");
            match ok {
                None => {
                    quote! {
                        #(#wrapper_pre_bindings)*
                        let __err = unsafe { #ffi_name(#handle_arg #(#wrapper_args),*) };
                        if __err.code == 0 { Ok(()) } else { Err(#err_ty::from_ffi(__err)) }
                    }
                }
                Some(vk) => {
                    let (out_decl, out_ptr, ok_convert) = client_result_ok_from_ffi(vk, rust_ret);
                    quote! {
                        #(#wrapper_pre_bindings)*
                        #out_decl
                        let __err = unsafe { #ffi_name(#handle_arg #(#wrapper_args,)* #out_ptr) };
                        if __err.code == 0 { Ok(#ok_convert) } else { Err(#err_ty::from_ffi(__err)) }
                    }
                }
            }
        }
    }
}

/// Convert a raw FFI return value to the Rust type for Value returns.
fn client_value_from_ffi(_vk: &MetaValueKind, rust_ret: &TokenStream2) -> TokenStream2 {
    quote! { <#rust_ret as ffier::FfiType>::from_c(__raw) }
}

/// For Result<T, E> returns, build (out_decl, out_ptr_expr, ok_convert).
fn client_result_ok_from_ffi(
    _vk: &MetaValueKind,
    rust_ret: &TokenStream2,
) -> (TokenStream2, TokenStream2, TokenStream2) {
    let ok_type = ffier_gen_c::extract_result_ok_type(rust_ret);
    (
        quote! { let mut __out = std::mem::MaybeUninit::uninit(); },
        quote! { __out.as_mut_ptr() },
        quote! { <#ok_type as ffier::FfiType>::from_c(unsafe { __out.assume_init() }) },
    )
}


// ===========================================================================
// Trait impl client generation
// ===========================================================================

fn generate_trait_impl_client(meta: MetaTraitImpl, emit_trait_def: bool) -> TokenStream2 {
    let trait_name = &meta.trait_name;
    let struct_name = &meta.struct_name;
    let fn_pfx = meta.fn_pfx();
    let struct_snake = camel_to_snake(&struct_name.to_string());

    let impl_generics = if meta.lifetimes.is_empty() {
        quote! {}
    } else {
        let lts: Vec<_> = meta
            .lifetimes
            .iter()
            .map(|lt| syn::Lifetime::new(&format!("'{lt}"), proc_macro2::Span::call_site()))
            .collect();
        quote! { <#(#lts),*> }
    };
    let trait_with_lts = if meta.trait_lifetime_args.is_empty() {
        quote! { #trait_name }
    } else {
        let lts: Vec<_> = meta
            .trait_lifetime_args
            .iter()
            .map(|lt| syn::Lifetime::new(&format!("'{lt}"), proc_macro2::Span::call_site()))
            .collect();
        quote! { #trait_name<#(#lts),*> }
    };
    let struct_with_lts = if meta.lifetimes.is_empty() {
        quote! { #struct_name }
    } else {
        let lts: Vec<_> = meta
            .lifetimes
            .iter()
            .map(|lt| syn::Lifetime::new(&format!("'{lt}"), proc_macro2::Span::call_site()))
            .collect();
        quote! { #struct_name<#(#lts),*> }
    };

    let trait_def = if !emit_trait_def {
        quote! {}
    } else {
        let method_sigs: Vec<_> = meta
            .methods
            .iter()
            .map(|m| {
                let mname = &m.name;
                let params: Vec<_> = m
                    .params
                    .iter()
                    .map(|p| {
                        let id = &p.name;
                        let rt = &p.rust_type;
                        quote! { #id: #rt }
                    })
                    .collect();
                let ret = match &m.ret {
                    MetaVtableRet::Void => quote! {},
                    MetaVtableRet::Value { rust_type, .. } => quote! { -> #rust_type },
                };
                quote! { fn #mname(&self, #(#params),*) #ret; }
            })
            .collect();

        // Trait definition generics: only declared lifetime params, not concrete
        // ones like 'static.
        let trait_def_generics = {
            let lts: Vec<_> = meta
                .trait_lifetime_args
                .iter()
                .filter(|lt| *lt != "static")
                .map(|lt| syn::Lifetime::new(&format!("'{lt}"), proc_macro2::Span::call_site()))
                .collect();
            if lts.is_empty() {
                quote! {}
            } else {
                quote! { <#(#lts),*> }
            }
        };

        quote! {
            pub trait #trait_name #trait_def_generics {
                #(#method_sigs)*

                #[doc(hidden)]
                fn __into_raw_handle(self) -> *mut core::ffi::c_void where Self: Sized;
            }
        }
    };

    // Extern declarations for trait method C functions
    let extern_decls: Vec<_> = meta
        .methods
        .iter()
        .map(|m| {
            let ffi_name = format_ident!("{fn_pfx}{struct_snake}_{}", m.name);
            let params: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let id = &p.name;
                    let bt = &p.bridge_type;
                    quote! { #id: <#bt as ffier::FfiType>::CRepr }
                })
                .collect();
            let ret = match &m.ret {
                MetaVtableRet::Void => quote! {},
                MetaVtableRet::Value { bridge_type, .. } => {
                    quote! { -> <#bridge_type as ffier::FfiType>::CRepr }
                }
            };
            quote! { pub fn #ffi_name(handle: *mut core::ffi::c_void #(, #params)*) #ret; }
        })
        .collect();

    // Trait method implementations calling through C ABI
    let trait_method_impls: Vec<_> = meta
        .methods
        .iter()
        .map(|m| {
            let method_name = &m.name;
            let ffi_name = format_ident!("{fn_pfx}{struct_snake}_{method_name}");

            let params: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let id = &p.name;
                    let rt = &p.rust_type;
                    quote! { #id: #rt }
                })
                .collect();

            let ret = match &m.ret {
                MetaVtableRet::Void => quote! {},
                MetaVtableRet::Value { rust_type, .. } => quote! { -> #rust_type },
            };

            // Convert Rust args to C repr for the call
            let call_args: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let id = &p.name;
                    let rt = &p.rust_type;
                    quote! { <#rt as ffier::FfiType>::into_c(#id) }
                })
                .collect();

            // Convert C return to Rust type
            let body = match &m.ret {
                MetaVtableRet::Void => {
                    quote! { unsafe { #ffi_name(self.0, #(#call_args),*) } }
                }
                MetaVtableRet::Value { rust_type, .. } => {
                    quote! {
                        let __raw = unsafe { #ffi_name(self.0, #(#call_args),*) };
                        <#rust_type as ffier::FfiType>::from_c(__raw)
                    }
                }
            };

            quote! {
                fn #method_name(&self, #(#params),*) #ret {
                    #body
                }
            }
        })
        .collect();

    quote! {
        #trait_def

        unsafe extern "C" {
            #(#extern_decls)*
        }

        impl #impl_generics #trait_with_lts for #struct_with_lts {
            #(#trait_method_impls)*

            fn __into_raw_handle(self) -> *mut core::ffi::c_void {
                let this = std::mem::ManuallyDrop::new(self);
                this.0
            }
        }
    }
}
