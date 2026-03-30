//! Client code generation from parsed metadata.
//!
//! `generate_client` takes a metadata token stream (starting with `@exportable`,
//! `@error`, or `@implementable`) and produces safe Rust wrapper code that calls
//! through C ABI extern declarations.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use ffier_meta::{
    FfiRepr, MetaError, MetaExportable, MetaImplementable, MetaParamKind, MetaReceiver, MetaReturn,
    MetaValueKind, MetaVtableParamType, MetaVtableRetType, camel_to_snake, camel_to_upper_snake,
    peek_meta_tag,
};

/// Generates Rust client source code as a named `&str` constant.
#[proc_macro]
pub fn generate_client_source(input: TokenStream) -> TokenStream {
    generate_client_source_impl(input.into()).into()
}

/// Generates client code (safe Rust wrappers calling through extern "C") from metadata.
///
/// The input token stream must start with one of:
/// - `@exportable, ...` --- generates wrapper struct with safe methods
/// - `@error, ...` --- generates error enum with `from_ffi`, Display, Error
/// - `@implementable, ...` --- generates vtable struct + handle wrapper
fn generate_client_impl(input: TokenStream2) -> TokenStream2 {
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
        "implementable" => {
            let meta: MetaImplementable = match syn::parse2(input) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_implementable_client(meta)
        }
        _ => {
            let msg = format!(
                "unknown metadata tag `@{tag}`: expected @exportable, @error, or @implementable"
            );
            quote! { compile_error!(#msg); }
        }
    }
}

/// Same as `generate_client_impl` but wraps the output in a string literal.
/// Like `generate_client_impl`, but wraps the output in a `const` declaration
/// containing the source code as a string literal.
///
/// Emits: `const FFIER_SRC_{TYPE_UPPER}: &str = "...";`
/// The const name is derived from the type name in the metadata.
fn generate_client_source_impl(input: TokenStream2) -> TokenStream2 {
    let tag = peek_meta_tag(&input);
    let type_name = peek_meta_name(&input);
    let upper_name = camel_to_upper_snake(&type_name);
    let const_name = if tag == "implementable" {
        format_ident!("FFIER_SRC_VTABLE_{upper_name}")
    } else {
        format_ident!("FFIER_SRC_{upper_name}")
    };

    let code = generate_client_impl(input);
    let source = code.to_string();
    quote! { const #const_name: &str = #source; }
}

/// Peek at the type/trait name from a metadata token stream.
///
/// Looks for `name = IDENT` or `trait_name = IDENT` and returns the IDENT.
fn peek_meta_name(input: &TokenStream2) -> String {
    let tokens: Vec<proc_macro2::TokenTree> = input.clone().into_iter().collect();
    for i in 0..tokens.len().saturating_sub(2) {
        if let proc_macro2::TokenTree::Ident(ref id) = tokens[i]
            && (id == "name" || id == "trait_name")
            && let proc_macro2::TokenTree::Punct(ref p) = tokens[i + 1]
            && p.as_char() == '='
            && let proc_macro2::TokenTree::Ident(ref name) = tokens[i + 2]
        {
            return name.to_string();
        }
    }
    "Unknown".to_string()
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

    // FfiType impl --- only if no lifetimes
    let client_ffi_type_impl = if !has_lifetimes {
        quote! {
            impl ffier::FfiType for #struct_name {
                type CRepr = *mut core::ffi::c_void;
                const C_TYPE_NAME: &str = "";
                fn into_c(self) -> *mut core::ffi::c_void { self.__into_raw() }
                fn from_c(repr: *mut core::ffi::c_void) -> Self { Self::__from_raw(repr) }
            }
        }
    } else {
        quote! {}
    };

    // Destroy function
    let destroy_name_str = format!("{}{}_destroy", fn_pfx, struct_lower);
    let destroy_ident = format_ident!("{destroy_name_str}");

    let mut client_extern_decls = Vec::new();
    let mut client_methods = Vec::new();
    let mut client_dyn_traits: Vec<TokenStream2> = Vec::new();

    // Destroy extern decl
    client_extern_decls.push(quote! {
        fn #destroy_ident(handle: *mut core::ffi::c_void);
    });

    for m in &meta.methods {
        let ffi_name = format_ident!("{}{}", fn_pfx, m.ffi_name);
        let method_name = &m.name;

        let has_receiver = m.receiver != MetaReceiver::None;
        let is_mut = m.receiver == MetaReceiver::Mut;
        let is_by_value = m.receiver == MetaReceiver::Value;
        let is_builder = m.is_builder;
        let handle_is_indirect = is_builder && is_by_value;

        // --- Build extern "C" declaration ---
        let extern_handle_param = if has_receiver {
            if handle_is_indirect {
                Some(quote! { handle: *mut *mut core::ffi::c_void, })
            } else {
                Some(quote! { handle: *mut core::ffi::c_void, })
            }
        } else {
            None
        };

        let extern_params: Vec<_> = m
            .params
            .iter()
            .map(|p| client_extern_param_tokens(&p.name, &p.kind))
            .collect();

        // Return type + out param for extern decl
        let (extern_ret, extern_out_param) = match &m.ret {
            MetaReturn::Void => (quote! {}, None),
            MetaReturn::Value(vk) => {
                let ann = client_extern_value_ret_annotation(vk);
                (ann, None)
            }
            MetaReturn::Result { ok, .. } => {
                let out = ok.as_ref().map(|vk| {
                    let ty = ffier_gen_c::c_out_param_type(vk);
                    quote! { result: #ty, }
                });
                (quote! { -> ffier::FfierError }, out)
            }
        };

        client_extern_decls.push(quote! {
            fn #ffi_name(#extern_handle_param #(#extern_params,)* #extern_out_param) #extern_ret;
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
                    MetaParamKind::SliceStr => quote! { #id: &str },
                    MetaParamKind::SliceBytes => quote! { #id: &[u8] },
                    MetaParamKind::SlicePath => quote! { #id: &std::path::Path },
                    MetaParamKind::StrSlice => quote! { #id: &[&str] },
                    MetaParamKind::HandleRef { .. } => {
                        let rust_type =
                            p.rust_type.as_ref().expect("HandleRef must have rust_type");
                        quote! { #id: #rust_type }
                    }
                    MetaParamKind::DynDispatch { c_name_suffix, .. } => {
                        let trait_name = format_ident!("Into{c_name_suffix}Handle");
                        quote! { #id: impl #trait_name }
                    }
                    MetaParamKind::Regular { .. } => {
                        let rust_type = p
                            .rust_type
                            .as_ref()
                            .expect("Regular param must have rust_type");
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
                    MetaParamKind::SliceStr => quote! { ffier::FfierBytes::from_str(#id) },
                    MetaParamKind::SliceBytes => quote! { ffier::FfierBytes::from_bytes(#id) },
                    MetaParamKind::SlicePath => quote! { ffier::FfierBytes::from_path(#id) },
                    MetaParamKind::StrSlice => {
                        quote! { __ffi_strs.as_ptr(), __ffi_strs.len() }
                    }
                    MetaParamKind::HandleRef { .. } => quote! { #id.0 },
                    MetaParamKind::DynDispatch { .. } => {
                        quote! { #id.into_raw_handle() }
                    }
                    MetaParamKind::Regular { repr, .. } => match repr {
                        FfiRepr::Primitive => quote! { #id },
                        FfiRepr::Handle => {
                            let rust_type = p.rust_type.as_ref().unwrap();
                            quote! { <#rust_type as ffier::FfiType>::into_c(#id) }
                        }
                        FfiRepr::Other(_) => {
                            let rust_type = p.rust_type.as_ref().unwrap();
                            quote! { <#rust_type as ffier::FfiType>::into_c(#id) }
                        }
                    },
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
                            .map(|s| ffier::FfierBytes::from_str(s))
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

        client_methods.push(quote! {
            #(#doc_attrs)*
            pub fn #method_name(#wrapper_receiver #(#wrapper_params),*) #wrapper_ret_type {
                #wrapper_body
            }
        });

        // Generate dyn_param traits
        for p in &m.params {
            if let MetaParamKind::DynDispatch {
                c_name_suffix,
                variants,
            } = &p.kind
            {
                let trait_name = format_ident!("Into{c_name_suffix}Handle");
                let variant_impls: Vec<_> = variants
                    .iter()
                    .map(|(variant_name, _)| {
                        let variant_struct = format_ident!("{variant_name}");
                        quote! {
                            impl #trait_name for #variant_struct {
                                fn into_raw_handle(self) -> *mut core::ffi::c_void {
                                    let this = std::mem::ManuallyDrop::new(self);
                                    this.0
                                }
                            }
                        }
                    })
                    .collect();
                client_dyn_traits.push(quote! {
                    pub trait #trait_name {
                        fn into_raw_handle(self) -> *mut core::ffi::c_void;
                    }
                    #(#variant_impls)*
                });
            }
        }
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

        #(#client_dyn_traits)*
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
                .map(|(_id, vpt)| vtable_param_c_type(vpt))
                .collect();
            let ret = vtable_ret_c_type(&m.ret);
            quote! {
                pub #mname: unsafe extern "C" fn(
                    *mut core::ffi::c_void
                    #(, #params)*
                ) #ret,
            }
        })
        .collect();

    quote! {
        #[repr(C)]
        pub struct #vtable_struct_name {
            #(#vtable_field_defs)*
            #(#vtable_method_fields)*
            pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
        }

        unsafe extern "C" {
            fn #constructor_name(
                user_data: *mut core::ffi::c_void,
                vtable: *const #vtable_struct_name,
            ) -> *mut core::ffi::c_void;
        }

        pub struct #wrapper_name(*mut core::ffi::c_void);

        impl #wrapper_name {
            pub fn new(user_data: *mut core::ffi::c_void, vtable: &#vtable_struct_name) -> Self {
                Self(unsafe { #constructor_name(user_data, vtable) })
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

/// Generate extern "C" parameter tokens for client-side declarations.
/// Uses CONCRETE types, not `<T as FfiType>::CRepr`.
fn client_extern_param_tokens(id: &syn::Ident, kind: &MetaParamKind) -> TokenStream2 {
    let ty = ffier_gen_c::c_param_type(kind);
    if matches!(kind, MetaParamKind::StrSlice) {
        let len_id = format_ident!("{id}_len");
        quote! { #id: #ty, #len_id: usize }
    } else {
        quote! { #id: #ty }
    }
}

fn client_extern_value_ret_annotation(kind: &MetaValueKind) -> TokenStream2 {
    let ty = ffier_gen_c::c_return_type(kind);
    quote! { -> #ty }
}

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
fn client_value_from_ffi(vk: &MetaValueKind, rust_ret: &TokenStream2) -> TokenStream2 {
    match vk {
        MetaValueKind::Regular { repr, .. } => match repr {
            FfiRepr::Primitive => quote! { __raw },
            FfiRepr::Handle => {
                let rust_ret_str = rust_ret.to_string();
                if rust_ret_str.contains('\'') {
                    quote! { <#rust_ret>::__from_raw(__raw) }
                } else {
                    quote! { <#rust_ret as ffier::FfiType>::from_c(__raw) }
                }
            }
            FfiRepr::Other(_) => {
                quote! { <#rust_ret as ffier::FfiType>::from_c(__raw) }
            }
        },
        MetaValueKind::SliceStr => quote! {
            unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(__raw.data, __raw.len)) }
        },
        MetaValueKind::SliceBytes => quote! {
            unsafe { core::slice::from_raw_parts(__raw.data, __raw.len) }
        },
        MetaValueKind::SlicePath => quote! {
            unsafe { __raw.as_path() }
        },
    }
}

/// For Result<T, E> returns, build (out_decl, out_ptr_expr, ok_convert).
fn client_result_ok_from_ffi(
    vk: &MetaValueKind,
    rust_ret: &TokenStream2,
) -> (TokenStream2, TokenStream2, TokenStream2) {
    match vk {
        MetaValueKind::Regular { repr, .. } => match repr {
            FfiRepr::Primitive => (
                quote! { let mut __out = std::mem::MaybeUninit::uninit(); },
                quote! { __out.as_mut_ptr() },
                quote! { unsafe { __out.assume_init() } },
            ),
            FfiRepr::Handle => {
                let ok_type = extract_ok_type_from_tokens(rust_ret);
                let ok_type_str = ok_type.to_string();
                let convert = if ok_type_str.contains('\'') {
                    quote! { <#ok_type>::__from_raw(unsafe { __out.assume_init() }) }
                } else {
                    quote! { <#ok_type as ffier::FfiType>::from_c(unsafe { __out.assume_init() }) }
                };
                (
                    quote! { let mut __out = std::mem::MaybeUninit::uninit(); },
                    quote! { __out.as_mut_ptr() },
                    convert,
                )
            }
            FfiRepr::Other(_) => {
                let ok_type = extract_ok_type_from_tokens(rust_ret);
                let convert =
                    quote! { <#ok_type as ffier::FfiType>::from_c(unsafe { __out.assume_init() }) };
                (
                    quote! { let mut __out = std::mem::MaybeUninit::uninit(); },
                    quote! { __out.as_mut_ptr() },
                    convert,
                )
            }
        },
        MetaValueKind::SliceStr => (
            quote! { let mut __out = ffier::FfierBytes::EMPTY; },
            quote! { &mut __out },
            quote! { unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(__out.data, __out.len)) } },
        ),
        MetaValueKind::SliceBytes => (
            quote! { let mut __out = ffier::FfierBytes::EMPTY; },
            quote! { &mut __out },
            quote! { unsafe { core::slice::from_raw_parts(__out.data, __out.len) } },
        ),
        MetaValueKind::SlicePath => (
            quote! { let mut __out = ffier::FfierBytes::EMPTY; },
            quote! { &mut __out },
            quote! { unsafe { __out.as_path() } },
        ),
    }
}

/// Extract the Ok type from `Result<OkType, ErrType>` tokens.
fn extract_ok_type_from_tokens(tokens: &TokenStream2) -> TokenStream2 {
    // Parse as a type and extract Result's first generic arg
    if let Ok(ty) = syn::parse2::<syn::Type>(tokens.clone())
        && let syn::Type::Path(tp) = &ty
        && let Some(last) = tp.path.segments.last()
        && last.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(syn::GenericArgument::Type(ok_ty)) = args.args.first()
    {
        return quote! { #ok_ty };
    }
    // Fallback: return the whole thing (shouldn't happen)
    tokens.clone()
}

/// Generate C type tokens for vtable parameter types (client side).
fn vtable_param_c_type(vpt: &MetaVtableParamType) -> TokenStream2 {
    match vpt {
        MetaVtableParamType::Primitive(ty) => quote! { #ty },
        MetaVtableParamType::Str | MetaVtableParamType::Bytes | MetaVtableParamType::Path => {
            quote! { ffier::FfierBytes }
        }
        MetaVtableParamType::Handle(_) => quote! { *mut core::ffi::c_void },
    }
}

/// Generate C return type annotation for vtable return types (client side).
fn vtable_ret_c_type(ret: &MetaVtableRetType) -> TokenStream2 {
    match ret {
        MetaVtableRetType::Void => quote! {},
        MetaVtableRetType::Primitive(ty) => quote! { -> #ty },
        MetaVtableRetType::Str | MetaVtableRetType::Bytes | MetaVtableRetType::Path => {
            quote! { -> ffier::FfierBytes }
        }
        MetaVtableRetType::Handle(_) => quote! { -> *mut core::ffi::c_void },
    }
}
