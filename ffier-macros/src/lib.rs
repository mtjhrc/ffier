use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Data, DeriveInput, FnArg, GenericArgument, ImplItem, ItemImpl, ItemTrait, LitStr, Pat,
    PathArguments, ReturnType, Token, TraitItem, Type, parse::Parse, parse_macro_input,
    visit_mut::VisitMut,
};

struct ReflectArgs {
    prefix: Option<String>,
}

impl Parse for ReflectArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self { prefix: None });
        }
        let ident: syn::Ident = input.parse()?;
        if ident != "prefix" {
            return Err(syn::Error::new(ident.span(), "expected `prefix`"));
        }
        input.parse::<Token![=]>()?;
        let lit: LitStr = input.parse()?;
        Ok(Self {
            prefix: Some(lit.value()),
        })
    }
}

// ---------------------------------------------------------------------------
// Type classification for params and return values
// ---------------------------------------------------------------------------

enum SliceKind {
    Str,
    Bytes,
    Path,
}

enum ParamKind {
    Regular(proc_macro2::TokenStream),
    Slice(SliceKind),
    /// `&[&str]` — slice of string references, expands to two C params.
    StrSlice,
    /// `&ExportedType` — borrows an opaque handle.
    HandleRef {
        inner_ty: proc_macro2::TokenStream,
        is_mut: bool,
    },
    /// `impl Trait` param with runtime dispatch over listed concrete types.
    DynDispatch(DynParamConfig),
}

struct DynParamConfig {
    /// C type name suffix (e.g. "Device" → "{Prefix}Device" typedef)
    c_name: String,
    /// Concrete types to dispatch over, as token streams (cross-crate safe)
    variants: Vec<(String, proc_macro2::TokenStream)>, // (ident_name, tokens)
}

enum ValueKind {
    Regular(proc_macro2::TokenStream),
    Slice(SliceKind),
}

enum ReturnKind {
    Void,
    Value(ValueKind),
    Result {
        ok_ty: Option<ValueKind>,
        #[allow(dead_code)]
        err_ty: proc_macro2::TokenStream,
        err_ident: String,
    },
}

struct MethodInfo {
    method_name: syn::Ident,
    ffi_name: syn::Ident,
    ffi_name_str: String,
    has_receiver: bool,
    is_mut: bool,
    is_by_value: bool,
    /// True if this method returns Self (builder pattern) — C gets void but
    /// the bridge macro writes the returned Self back into the handle.
    is_builder: bool,
    param_idents: Vec<syn::Ident>,
    param_name_strs: Vec<String>,
    param_kinds: Vec<ParamKind>,
    /// Original Rust parameter types (lifetime-erased, Self-replaced) for client codegen.
    param_orig_types: Vec<Type>,
    ret: ReturnKind,
    /// Original Rust return type for client codegen.
    ret_orig_type: ReturnType,
    doc_lines: Vec<String>,
}

/// Detect `&[&str]` — a slice of string references.
fn is_str_slice(ty: &Type) -> bool {
    let Type::Reference(ref_ty) = ty else {
        return false;
    };
    let Type::Slice(sl) = &*ref_ty.elem else {
        return false;
    };
    let Type::Reference(inner_ref) = &*sl.elem else {
        return false;
    };
    matches!(&*inner_ref.elem, Type::Path(tp) if tp.path.is_ident("str"))
}

/// Detect `&str`, `&[u8]`, `&Path` reference types.
fn classify_ref_type(ty: &Type) -> Option<SliceKind> {
    let Type::Reference(ref_ty) = ty else {
        return None;
    };
    match &*ref_ty.elem {
        Type::Path(tp) if tp.path.is_ident("str") => Some(SliceKind::Str),
        Type::Path(tp) => {
            let last = tp.path.segments.last()?;
            if last.ident == "Path" {
                Some(SliceKind::Path)
            } else {
                None
            }
        }
        Type::Slice(sl) => {
            if let Type::Path(tp) = &*sl.elem {
                if tp.path.is_ident("u8") {
                    return Some(SliceKind::Bytes);
                }
            }
            None
        }
        _ => None,
    }
}

fn classify_value(
    ty: &Type,
    reexport_types: &mut Vec<Type>,
    reexport_aliases: &mut Vec<syn::Ident>,
    alias_counter: &mut u32,
    helper_mod: &syn::Ident,
) -> ValueKind {
    if let Some(sk) = classify_ref_type(ty) {
        ValueKind::Slice(sk)
    } else {
        ValueKind::Regular(type_tokens_for_macro(
            ty,
            reexport_types,
            reexport_aliases,
            alias_counter,
            helper_mod,
        ))
    }
}

// ---------------------------------------------------------------------------
// Code generation helpers
// ---------------------------------------------------------------------------

fn ffi_param_tokens(id: &syn::Ident, kind: &ParamKind) -> proc_macro2::TokenStream {
    match kind {
        ParamKind::Regular(ty) => quote! { #id: <#ty as ffier::FfiType>::CRepr },
        ParamKind::Slice(_) => quote! { #id: ffier::FfierBytes },
        ParamKind::StrSlice => {
            let len_id = format_ident!("{id}_len");
            quote! { #id: *const ffier::FfierBytes, #len_id: usize }
        }
        ParamKind::HandleRef { .. } | ParamKind::DynDispatch(_) => {
            quote! { #id: *mut core::ffi::c_void }
        }
    }
}

fn param_conversion(id: &syn::Ident, kind: &ParamKind) -> proc_macro2::TokenStream {
    // Slice/HandleRef conversions produce references with unconstrained lifetimes
    // (from raw pointers). The compiler infers the needed lifetime — typically
    // 'static at the FFI boundary. The C caller is responsible for keeping the
    // data alive.
    match kind {
        ParamKind::Regular(ty) => quote! { <#ty as ffier::FfiType>::from_c(#id) },
        ParamKind::Slice(SliceKind::Str) => quote! { unsafe {
            core::str::from_utf8_unchecked(
                core::slice::from_raw_parts(#id.data, #id.len))
        } },
        ParamKind::Slice(SliceKind::Bytes) => quote! { unsafe {
            core::slice::from_raw_parts(#id.data, #id.len)
        } },
        ParamKind::Slice(SliceKind::Path) => quote! { unsafe { #id.as_path() } },
        ParamKind::StrSlice => {
            let len_id = format_ident!("{id}_len");
            quote! { {
                let __slice = unsafe { core::slice::from_raw_parts(#id, #len_id) };
                let __strs: Vec<&str> = __slice.iter().map(|b| unsafe {
                    core::str::from_utf8_unchecked(
                        core::slice::from_raw_parts(b.data, b.len))
                }).collect();
                __strs
            } }
        }
        ParamKind::HandleRef {
            inner_ty,
            is_mut: true,
        } => {
            quote! { unsafe {
                &mut (*(#id as *mut ffier::FfierTaggedBox<#inner_ty>)).value
            } }
        }
        ParamKind::HandleRef {
            inner_ty,
            is_mut: false,
        } => {
            quote! { unsafe {
                &(*(#id as *const ffier::FfierTaggedBox<#inner_ty>)).value
            } }
        }
        ParamKind::DynDispatch(_) => {
            // Dispatch is handled specially in the method codegen, not here.
            // This is a placeholder — the actual match is generated inline.
            quote! { compile_error!("DynDispatch should not use param_conversion") }
        }
    }
}

fn param_c_type_expr(
    kind: &ParamKind,
    str_name: &str,
    bytes_name: &str,
    path_name: &str,
) -> proc_macro2::TokenStream {
    match kind {
        ParamKind::Regular(ty) => quote! { <#ty as ffier::FfiType>::C_TYPE_NAME },
        ParamKind::Slice(SliceKind::Str) => quote! { #str_name },
        ParamKind::Slice(SliceKind::Bytes) => quote! { #bytes_name },
        ParamKind::Slice(SliceKind::Path) => quote! { #path_name },
        ParamKind::StrSlice => {
            // StrSlice is handled by expanding to two entries in c_type_exprs directly;
            // this arm should not be reached.
            quote! { compile_error!("StrSlice should not use param_c_type_expr") }
        }
        ParamKind::HandleRef { inner_ty, .. } => {
            quote! { <#inner_ty as ffier::FfiHandle>::C_HANDLE_NAME }
        }
        ParamKind::DynDispatch(cfg) => {
            let name = &cfg.c_name;
            quote! { #name }
        }
    }
}

fn value_ret_annotation(kind: &ValueKind) -> proc_macro2::TokenStream {
    match kind {
        ValueKind::Regular(ty) => quote! { -> <#ty as ffier::FfiType>::CRepr },
        ValueKind::Slice(_) => quote! { -> ffier::FfierBytes },
    }
}

fn value_into_c(kind: &ValueKind, var: &syn::Ident) -> proc_macro2::TokenStream {
    match kind {
        ValueKind::Regular(ty) => quote! { <#ty as ffier::FfiType>::into_c(#var) },
        ValueKind::Slice(SliceKind::Str) => quote! { ffier::FfierBytes::from_str(#var) },
        ValueKind::Slice(SliceKind::Bytes) => quote! { ffier::FfierBytes::from_bytes(#var) },
        ValueKind::Slice(SliceKind::Path) => quote! { ffier::FfierBytes::from_path(#var) },
    }
}

fn value_c_type_expr(
    kind: &ValueKind,
    str_name: &str,
    bytes_name: &str,
    path_name: &str,
) -> proc_macro2::TokenStream {
    match kind {
        ValueKind::Regular(ty) => quote! { <#ty as ffier::FfiType>::C_TYPE_NAME },
        ValueKind::Slice(SliceKind::Str) => quote! { #str_name },
        ValueKind::Slice(SliceKind::Bytes) => quote! { #bytes_name },
        ValueKind::Slice(SliceKind::Path) => quote! { #path_name },
    }
}

// ---------------------------------------------------------------------------
// Main macro
// ---------------------------------------------------------------------------

#[proc_macro_attribute]
pub fn exportable(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ReflectArgs);
    let input = parse_macro_input!(item as ItemImpl);

    // Strip #[ffier(...)] attributes from methods before emitting the impl block
    let impl_block = {
        let mut block = input.clone();
        for item in &mut block.items {
            if let ImplItem::Fn(method) = item {
                method.attrs.retain(|a| !a.path().is_ident("ffier"));
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
    let self_ty_static = erase_lifetimes(self_ty);
    let impl_generics = &input.generics;
    let _ = impl_generics; // used later for lifetime detection
    let struct_name = struct_ident.to_string();
    let struct_lower = camel_to_snake(&struct_name);

    let fn_pfx = args
        .prefix
        .as_ref()
        .map(|p| format!("{p}_"))
        .unwrap_or_default();
    let type_pfx = args
        .prefix
        .as_ref()
        .map(|p| snake_to_pascal(p))
        .unwrap_or_default();
    let upper_pfx = args
        .prefix
        .as_ref()
        .map(|p| format!("{}_", p.to_ascii_uppercase()))
        .unwrap_or_default();

    let handle_c_name = format!("{type_pfx}{struct_name}");
    let bytes_c_name = format!("{type_pfx}Bytes");
    let str_c_name = format!("{type_pfx}Str");
    let path_c_name = format!("{type_pfx}Path");
    let str_macro_name = format!("{upper_pfx}STR");

    let trait_path = input.trait_.as_ref().map(|(_, path, _)| path);

    let mut ffi_fns = Vec::new();
    let handle_typedef_expr = quote! { concat!("typedef void* ", #handle_c_name, ";") };
    let mut shared_types_exprs: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut decl_exprs: Vec<proc_macro2::TokenStream> = Vec::new();

    let mut reexport_types: Vec<Type> = Vec::new();
    let mut reexport_aliases: Vec<syn::Ident> = Vec::new();
    let helper_mod_name = format_ident!("_ffier_{struct_lower}");

    let mut methods = Vec::new();
    let mut alias_counter = 0u32;
    let mut uses_slices = false;
    let is_inherent = input.trait_.is_none();
    let mut warnings = Vec::new();

    for item in &input.items {
        let ImplItem::Fn(method) = item else { continue };
        let method_name = &method.sig.ident;
        let ffi_name = format_ident!("{fn_pfx}{struct_lower}_{method_name}");
        let ffi_name_str = ffi_name.to_string();

        let self_arg = method.sig.inputs.first();
        let has_receiver = matches!(self_arg, Some(FnArg::Receiver(_)));

        // Skip non-public methods in inherent impls (bridge crate can't call them)
        if is_inherent && !matches!(method.vis, syn::Visibility::Public(_)) {
            let msg = format!(
                "ffier: skipping non-public method `{}`; make it `pub` to export via FFI",
                method_name
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
        let (is_mut, is_by_value) = if has_receiver {
            let receiver = match self_arg.unwrap() {
                FnArg::Receiver(r) => r,
                _ => unreachable!(),
            };
            (receiver.mutability.is_some(), receiver.reference.is_none())
        } else {
            (false, false)
        };

        // Parse #[ffier(dyn_param(param, "CName", [Type1, Type2]))] on this method
        let dyn_params = parse_dyn_param_attrs(
            &method.attrs,
            &mut reexport_types,
            &mut reexport_aliases,
            &mut alias_counter,
            &helper_mod_name,
            &type_pfx,
        );

        let mut param_idents = Vec::new();
        let mut param_name_strs = Vec::new();
        let mut param_kinds = Vec::new();
        let mut param_orig_types = Vec::new();

        let skip_n = if has_receiver { 1 } else { 0 };
        for arg in method.sig.inputs.iter().skip(skip_n) {
            let FnArg::Typed(pat_ty) = arg else { continue };
            let Pat::Ident(pat_ident) = &*pat_ty.pat else {
                continue;
            };
            let param_name = pat_ident.ident.to_string();
            param_idents.push(pat_ident.ident.clone());
            param_name_strs.push(param_name.clone());

            // Capture original type (Self-replaced, lifetimes preserved) for client codegen
            let param_ty_orig = replace_self_type(&pat_ty.ty, self_ty);
            param_orig_types.push(param_ty_orig);

            // Check if this param has a dyn_param annotation
            if let Some(cfg) = dyn_params.iter().find(|d| d.0 == param_name) {
                param_kinds.push(ParamKind::DynDispatch(DynParamConfig {
                    c_name: cfg.1.clone(),
                    variants: cfg.2.clone(),
                }));
                continue;
            }

            // Replace `Self` with the concrete (lifetime-erased) struct type
            let param_ty = replace_self_type(&pat_ty.ty, &self_ty_static);

            let kind = if is_str_slice(&param_ty) {
                uses_slices = true;
                ParamKind::StrSlice
            } else if let Some(sk) = classify_ref_type(&param_ty) {
                uses_slices = true;
                ParamKind::Slice(sk)
            } else if let Type::Reference(ref_ty) = &param_ty {
                // &SomeType / &mut SomeType that isn't str/[u8]/Path → handle ref
                let inner_ty = type_tokens_for_macro(
                    &ref_ty.elem,
                    &mut reexport_types,
                    &mut reexport_aliases,
                    &mut alias_counter,
                    &helper_mod_name,
                );
                ParamKind::HandleRef {
                    inner_ty,
                    is_mut: ref_ty.mutability.is_some(),
                }
            } else {
                ParamKind::Regular(type_tokens_for_macro(
                    &param_ty,
                    &mut reexport_types,
                    &mut reexport_aliases,
                    &mut alias_counter,
                    &helper_mod_name,
                ))
            };
            param_kinds.push(kind);
        }

        // Detect builder pattern: method returns Self (by value or reference)
        let is_builder_return = if has_receiver {
            match &method.sig.output {
                ReturnType::Default => false,
                ReturnType::Type(_, ty) => {
                    let ty = &replace_self_type(ty, &self_ty_static);
                    is_self_return(ty, &self_ty_static)
                        || extract_result_types(ty)
                            .is_some_and(|(ok, _)| is_self_return(&ok, &self_ty_static))
                }
            }
        } else {
            false
        };

        let ret = match &method.sig.output {
            ReturnType::Default => ReturnKind::Void,
            ReturnType::Type(_, ty) => {
                // Replace `Self` and erase lifetimes for FFI boundary
                let ty = &replace_self_type(ty, &self_ty_static);

                // Builder pattern: self -> Self (or &mut self -> &mut Self)
                // generates void in C — the caller already has the handle.
                if is_builder_return && extract_result_types(ty).is_none() {
                    ReturnKind::Void
                } else if let Some((ok, err)) = extract_result_types(ty) {
                    let err_ident = type_ident_name(&err);
                    let err_tokens = type_tokens_for_macro(
                        &err,
                        &mut reexport_types,
                        &mut reexport_aliases,
                        &mut alias_counter,
                        &helper_mod_name,
                    );
                    // Result<Self, E> in builder context → treat as Result<(), E> for C
                    let ok_kind = if is_unit_type(&ok) || (is_builder_return && is_self_return(&ok, &self_ty_static)) {
                        None
                    } else {
                        let vk = classify_value(
                            &ok,
                            &mut reexport_types,
                            &mut reexport_aliases,
                            &mut alias_counter,
                            &helper_mod_name,
                        );
                        if matches!(vk, ValueKind::Slice(_)) {
                            uses_slices = true;
                        }
                        Some(vk)
                    };
                    ReturnKind::Result {
                        ok_ty: ok_kind,
                        err_ty: err_tokens,
                        err_ident,
                    }
                } else {
                    let vk = classify_value(
                        ty,
                        &mut reexport_types,
                        &mut reexport_aliases,
                        &mut alias_counter,
                        &helper_mod_name,
                    );
                    if matches!(vk, ValueKind::Slice(_)) {
                        uses_slices = true;
                    }
                    ReturnKind::Value(vk)
                }
            }
        };

        let doc_lines = extract_doc_comments(&method.attrs);

        methods.push(MethodInfo {
            method_name: method_name.clone(),
            ffi_name,
            ffi_name_str,
            has_receiver,
            is_mut,
            is_by_value,
            is_builder: is_builder_return,
            param_idents,
            param_name_strs,
            param_kinds,
            param_orig_types,
            ret,
            ret_orig_type: {
                let mut out = method.sig.output.clone();
                if let ReturnType::Type(_, ty) = &mut out {
                    *ty = Box::new(replace_self_type(ty, self_ty));
                }
                out
            },
            doc_lines,
        });
    }

    // Bytes/Str/Path struct + typedefs
    if uses_slices {
        let bytes_macro_name = format!("{upper_pfx}BYTES");

        // KrunStr — const char* (signedness-neutral, matches C string convention)
        shared_types_exprs.push(quote! {
            concat!("/* Caller must ensure data is valid UTF-8 */")
        });
        shared_types_exprs.push(quote! { "typedef struct {" });
        shared_types_exprs.push(quote! { "    const char* data;" });
        shared_types_exprs.push(quote! { "    uintptr_t len;" });
        shared_types_exprs.push(quote! { concat!("} ", #str_c_name, ";") });
        shared_types_exprs.push(quote! { "" });

        // KrunPath — same layout as KrunStr (const char*, UTF-8 path)
        shared_types_exprs.push(quote! {
            concat!("/* Caller must ensure data is a valid UTF-8 path */")
        });
        shared_types_exprs.push(quote! {
            concat!("typedef ", #str_c_name, " ", #path_c_name, ";")
        });
        shared_types_exprs.push(quote! { "" });

        // KrunBytes — const uint8_t* (always unsigned raw bytes)
        shared_types_exprs.push(quote! { "typedef struct {" });
        shared_types_exprs.push(quote! { "    const uint8_t* data;" });
        shared_types_exprs.push(quote! { "    uintptr_t len;" });
        shared_types_exprs.push(quote! { concat!("} ", #bytes_c_name, ";") });
        shared_types_exprs.push(quote! { "" });
        shared_types_exprs.push(quote! {
            concat!("#define ", #str_macro_name, "(s) ((", #str_c_name, "){ .data = (s), .len = strlen(s) })")
        });
        shared_types_exprs.push(quote! {
            concat!(
                "#define ", #bytes_macro_name, "(arr) ({ \\")
        });
        shared_types_exprs.push(quote! {
            concat!(
                "    _Static_assert( \\")
        });
        shared_types_exprs.push(quote! {
            concat!(
                "        !__builtin_types_compatible_p(typeof(arr), typeof(&(arr)[0])), \\")
        });
        shared_types_exprs.push(quote! {
            concat!(
                "        \"", #bytes_macro_name, "() requires an array, not a pointer\"); \\")
        });
        shared_types_exprs.push(quote! {
            concat!(
                "    ((", #bytes_c_name, "){ .data = (const uint8_t*)(arr), .len = sizeof(arr) }); \\")
        });
        shared_types_exprs.push(quote! { "})" });
    }

    // Per-error-type artifacts
    // Error type header declarations and FFI helpers are generated by the
    // FfiError derive's bridge macro, invoked once in the cdylib.

    // Generate typedefs for dyn_param dispatch types
    let mut generated_dyn_types: Vec<String> = Vec::new();
    for m in &methods {
        for (_, k) in m.param_idents.iter().zip(m.param_kinds.iter()) {
            if let ParamKind::DynDispatch(cfg) = k {
                if generated_dyn_types.contains(&cfg.c_name) {
                    continue;
                }
                generated_dyn_types.push(cfg.c_name.clone());

                let c_name = &cfg.c_name;

                // typedef void* KrunDevice; /* KrunFoo | KrunBar | ... */
                let variant_names: Vec<String> = cfg
                    .variants
                    .iter()
                    .map(|(name, _)| format!("{type_pfx}{name}"))
                    .collect();
                let variants_comment = variant_names.join(" | ");
                decl_exprs.push(quote! {
                    format!("typedef void* {}; /* {} */", #c_name, #variants_comment)
                });
            }
        }
    }

    // Method FFI functions
    for m in &methods {
        let ffi_name_str = &m.ffi_name_str;
        let ffi_name = &m.ffi_name;
        let method_name = &m.method_name;

        // Handle parameter: present for instance methods, absent for static.
        // Builder methods (by-value self returning Self) take a pointer to the
        // handle so the bridge can update it after the method returns a new Self.
        let handle_is_indirect = m.is_builder && m.is_by_value;
        let handle_type = if handle_is_indirect {
            format!("{handle_c_name}* handle")
        } else {
            format!("{handle_c_name} handle")
        };
        let handle_ffi_param = if m.has_receiver {
            if handle_is_indirect {
                Some(quote! { handle: *mut *mut core::ffi::c_void, })
            } else {
                Some(quote! { handle: *mut core::ffi::c_void, })
            }
        } else {
            None
        };

        // Self cast via FfierTaggedBox (instance methods only)
        let obj_binding = if m.has_receiver {
            let ffi_name_for_msg = &m.ffi_name_str;
            let handle_deref = if handle_is_indirect {
                quote! { let handle = unsafe { *handle }; }
            } else {
                quote! {}
            };
            let type_assert = quote! {
                #handle_deref
                let __actual = unsafe { ffier::handle_type_id(handle) };
                let __expected = <$struct_ty as ffier::FfiHandle>::type_id();
                assert!(
                    __actual == __expected,
                    "{}(): `handle` is not a {} (expected type_id={:?}, got {:?})",
                    #ffi_name_for_msg,
                    <$struct_ty as ffier::FfiHandle>::C_HANDLE_NAME,
                    __expected,
                    __actual,
                );
            };
            let cast = if m.is_by_value {
                quote! {
                    let tagged = *Box::from_raw(
                        handle as *mut ffier::FfierTaggedBox<$struct_ty>
                    );
                    tagged.value
                }
            } else if m.is_mut {
                quote! {
                    &mut (*(handle as *mut ffier::FfierTaggedBox<$struct_ty>)).value
                }
            } else {
                quote! {
                    &(*(handle as *const ffier::FfierTaggedBox<$struct_ty>)).value
                }
            };
            Some(quote! { #type_assert let obj = unsafe { #cast }; })
        } else {
            None
        };

        let ffi_params: Vec<_> = m
            .param_idents
            .iter()
            .zip(m.param_kinds.iter())
            .map(|(id, k)| ffi_param_tokens(id, k))
            .collect();

        let mut c_type_exprs = Vec::new();
        let mut header_param_names: Vec<String> = Vec::new();
        for (name, k) in m.param_name_strs.iter().zip(m.param_kinds.iter()) {
            if matches!(k, ParamKind::StrSlice) {
                let ptr_type = format!("const {str_c_name}*");
                c_type_exprs.push(quote! { #ptr_type });
                header_param_names.push(name.clone());
                c_type_exprs.push(quote! { "uintptr_t" });
                header_param_names.push(format!("{name}_len"));
            } else {
                c_type_exprs.push(param_c_type_expr(
                    k,
                    &str_c_name,
                    &bytes_c_name,
                    &path_c_name,
                ));
                header_param_names.push(name.clone());
            }
        }
        let param_name_str_refs: Vec<_> = header_param_names.iter().collect();

        // Check for DynDispatch params
        let dyn_dispatch = m
            .param_idents
            .iter()
            .zip(m.param_kinds.iter())
            .find_map(|(id, k)| match k {
                ParamKind::DynDispatch(cfg) => Some((id.clone(), cfg)),
                _ => None,
            });

        // Pre-bindings for multi-param types (e.g. StrSlice → Vec<&str>)
        let mut pre_bindings = Vec::new();
        // Build converted args: for DynDispatch params, use the raw ident
        // (dispatch match substitutes the correct conversion)
        let converted_args: Vec<_> = m
            .param_idents
            .iter()
            .zip(m.param_kinds.iter())
            .map(|(id, k)| match k {
                ParamKind::DynDispatch(_) => quote! { #id },
                ParamKind::StrSlice => {
                    let binding = param_conversion(id, k);
                    let vec_id = format_ident!("__{id}_vec");
                    pre_bindings.push(quote! { let #vec_id = #binding; });
                    quote! { &#vec_id }
                }
                other => param_conversion(id, other),
            })
            .collect();

        // Build the method call expression (without dispatch wrapping)
        let base_method_call = if m.has_receiver {
            if let Some(tp) = &trait_path {
                quote! { <$struct_ty as $crate::#tp>::#method_name(obj, #(#converted_args),*) }
            } else {
                quote! { obj.#method_name(#(#converted_args),*) }
            }
        } else {
            quote! { <$struct_ty>::#method_name(#(#converted_args),*) }
        };

        // Wrap in dispatch match if needed
        let method_call = if let Some((dyn_id, dyn_cfg)) = &dyn_dispatch {
            let if_branches: Vec<_> = dyn_cfg
                .variants
                .iter()
                .map(|(_, ty_tokens)| {
                    quote! {
                        if __type_id == <#ty_tokens as ffier::FfiHandle>::type_id() {
                            let #dyn_id = unsafe {
                                (*Box::from_raw(
                                    #dyn_id as *mut ffier::FfierTaggedBox<#ty_tokens>
                                )).value
                            };
                            #base_method_call
                        }
                    }
                })
                .collect();

            let variant_names: Vec<_> = dyn_cfg
                .variants
                .iter()
                .map(|(name, _)| name.as_str())
                .collect();
            let accepted_list = variant_names.join(" | ");
            let ffi_name_for_dispatch = &m.ffi_name_str;

            quote! {{
                let __type_id = unsafe { ffier::handle_type_id(#dyn_id) };
                #(#if_branches else)* {
                    panic!(
                        "{}(): parameter `{}` expected an object of type: {}, \
                         but got unknown handle (type_id={:?})",
                        #ffi_name_for_dispatch,
                        stringify!(#dyn_id),
                        #accepted_list,
                        __type_id,
                    );
                }
            }}
        } else {
            base_method_call
        };

        // Doxygen comment
        let (has_out_param, err_c_name_for_doc) = match &m.ret {
            ReturnKind::Result {
                ok_ty, err_ident, ..
            } => (ok_ty.is_some(), Some(format!("{type_pfx}{err_ident}"))),
            _ => (false, None),
        };
        if let Some(doc) = build_doxygen_comment(
            &m.doc_lines,
            &m.param_name_strs,
            has_out_param,
            err_c_name_for_doc.as_deref(),
        ) {
            decl_exprs.push(quote! { #doc });
        }

        let header_handle = if m.has_receiver {
            Some(&handle_type)
        } else {
            None
        };

        match &m.ret {
            ReturnKind::Void => {
                let header_line = build_header_line(
                    quote! { "void" },
                    ffi_name_str,
                    header_handle,
                    &c_type_exprs,
                    &param_name_str_refs,
                    None,
                );
                decl_exprs.push(header_line);

                // Builder pattern with by-value self: take ownership from handle,
                // call method (returns new Self), box it and write new handle back.
                let body = if handle_is_indirect {
                    quote! {
                        let handle_ptr = handle;
                        #obj_binding
                        #(#pre_bindings)*
                        let result = #method_call;
                        unsafe { *handle_ptr = <$struct_ty as ffier::FfiType>::into_c(result) };
                    }
                } else {
                    quote! {
                        #obj_binding
                        #(#pre_bindings)*
                        #method_call;
                    }
                };

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        #handle_ffi_param
                        #(#ffi_params),*
                    ) {
                        #body
                    }
                });
            }
            ReturnKind::Value(vk) => {
                let ret_c = value_c_type_expr(vk, &str_c_name, &bytes_c_name, &path_c_name);
                let header_line = build_header_line(
                    ret_c,
                    ffi_name_str,
                    header_handle,
                    &c_type_exprs,
                    &param_name_str_refs,
                    None,
                );
                decl_exprs.push(header_line);

                let ret_ann = value_ret_annotation(vk);
                let result_ident = format_ident!("result");
                let into_c = value_into_c(vk, &result_ident);

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        #handle_ffi_param
                        #(#ffi_params),*
                    ) #ret_ann {
                        #obj_binding
                        #(#pre_bindings)*
                        let result = #method_call;
                        #into_c
                    }
                });
            }
            ReturnKind::Result {
                ok_ty,
                err_ty: _,
                err_ident,
            } => {
                let err_c_name = format!("{type_pfx}{err_ident}");

                let out_c_type = ok_ty
                    .as_ref()
                    .map(|vk| value_c_type_expr(vk, &str_c_name, &bytes_c_name, &path_c_name));

                let header_line = build_header_line(
                    quote! { #err_c_name },
                    ffi_name_str,
                    header_handle,
                    &c_type_exprs,
                    &param_name_str_refs,
                    out_c_type.as_ref(),
                );
                decl_exprs.push(header_line);

                let ok_branch = match ok_ty {
                    Some(vk) => {
                        let ok_val_ident = format_ident!("ok_val");
                        let into_c = value_into_c(vk, &ok_val_ident);
                        quote! {
                            Ok(ok_val) => {
                                unsafe { result.write(#into_c) };
                                ffier::FfierError::ok()
                            }
                        }
                    }
                    None if handle_is_indirect => quote! {
                        Ok(new_self) => {
                            unsafe { *handle_ptr = <$struct_ty as ffier::FfiType>::into_c(new_self) };
                            ffier::FfierError::ok()
                        }
                    },
                    None => quote! {
                        Ok(_) => ffier::FfierError::ok(),
                    },
                };

                let out_ffi_param = ok_ty.as_ref().map(|vk| match vk {
                    ValueKind::Regular(ty) => {
                        quote! { result: *mut <#ty as ffier::FfiType>::CRepr, }
                    }
                    ValueKind::Slice(_) => {
                        quote! { result: *mut ffier::FfierBytes, }
                    }
                });

                let handle_ptr_binding = if handle_is_indirect {
                    quote! { let handle_ptr = handle; }
                } else {
                    quote! {}
                };

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        #handle_ffi_param
                        #(#ffi_params,)*
                        #out_ffi_param
                    ) -> ffier::FfierError {
                        #handle_ptr_binding
                        #obj_binding
                        #(#pre_bindings)*
                        match #method_call {
                            #ok_branch
                            Err(e) => ffier::FfierError::from_err(e),
                        }
                    }
                });
            }
        }
    }

    // destroy (no auto-generated create — methods returning Self serve as constructors)
    let destroy_name = format_ident!("{fn_pfx}{struct_lower}_destroy");
    let destroy_str = destroy_name.to_string();

    decl_exprs.push(quote! { concat!("void ", #destroy_str, "(", #handle_c_name, " handle);") });

    ffi_fns.push(quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #destroy_name(handle: *mut core::ffi::c_void) {
            if !handle.is_null() {
                let __actual = unsafe { ffier::handle_type_id(handle) };
                let __expected = <$struct_ty as ffier::FfiHandle>::type_id();
                assert!(
                    __actual == __expected,
                    "{}(): `handle` is not a {} (expected type_id={:?}, got {:?})",
                    #destroy_str,
                    <$struct_ty as ffier::FfiHandle>::C_HANDLE_NAME,
                    __expected,
                    __actual,
                );
                drop(unsafe {
                    Box::from_raw(handle as *mut ffier::FfierTaggedBox<$struct_ty>)
                });
            }
        }
    });

    let header_fn_name = format_ident!("{fn_pfx}{struct_lower}__header");
    let num_shared = shared_types_exprs.len();
    let num_decls = decl_exprs.len();
    let bridge_macro_name = format_ident!("{struct_lower}_ffier");

    let reexport_items: Vec<_> = reexport_types
        .iter()
        .zip(reexport_aliases.iter())
        .map(|(ty, alias)| {
            let erased = erase_lifetimes(ty);
            quote! { pub type #alias = super::#erased; }
        })
        .collect();

    // -----------------------------------------------------------------------
    // Client macro codegen — safe Rust wrappers calling through extern "C"
    // -----------------------------------------------------------------------

    let client_macro_name = format_ident!("{struct_lower}_ffi_client");
    let struct_ident_for_client = format_ident!("{struct_name}");
    let destroy_name_str = format!("{fn_pfx}{struct_lower}_destroy");
    let destroy_ident = format_ident!("{destroy_name_str}");

    // Extract lifetime params from the impl block for the client struct
    let impl_lifetimes: Vec<_> = input.generics.lifetimes().cloned().collect();
    let has_lifetimes = !impl_lifetimes.is_empty();
    let client_struct_generics = if has_lifetimes {
        let lts: Vec<_> = impl_lifetimes.iter().map(|lt| &lt.lifetime).collect();
        quote! { <#(#lts),*> }
    } else {
        quote! {}
    };
    let client_phantom = if has_lifetimes {
        let lts: Vec<_> = impl_lifetimes.iter().map(|lt| {
            let lt = &lt.lifetime;
            quote! { &#lt () }
        }).collect();
        quote! { , std::marker::PhantomData<(#(#lts),*)> }
    } else {
        quote! {}
    };
    let client_phantom_init = if has_lifetimes {
        quote! { , std::marker::PhantomData }
    } else {
        quote! {}
    };
    let client_ffi_type_impl = if !has_lifetimes {
        let si = &struct_ident_for_client;
        quote! {
            impl ffier::FfiType for #si {
                type CRepr = *mut core::ffi::c_void;
                const C_TYPE_NAME: &str = "";
                fn into_c(self) -> *mut core::ffi::c_void { self.__into_raw() }
                fn from_c(repr: *mut core::ffi::c_void) -> Self { Self::__from_raw(repr) }
            }
        }
    } else {
        quote! {}
    };

    let mut client_extern_decls = Vec::new();
    let mut client_methods = Vec::new();
    let mut client_dyn_traits = Vec::new();

    // extern decl for destroy
    client_extern_decls.push(quote! {
        fn #destroy_ident(handle: *mut core::ffi::c_void);
    });

    for m in &methods {
        let ffi_name = &m.ffi_name;
        let method_name = &m.method_name;
        let handle_is_indirect = m.is_builder && m.is_by_value;

        // --- Build extern "C" declaration ---
        let extern_handle_param = if m.has_receiver {
            if handle_is_indirect {
                Some(quote! { handle: *mut *mut core::ffi::c_void, })
            } else {
                Some(quote! { handle: *mut core::ffi::c_void, })
            }
        } else {
            None
        };

        let extern_params: Vec<_> = m.param_idents.iter()
            .zip(m.param_kinds.iter())
            .map(|(id, k)| ffi_param_tokens(id, k))
            .collect();

        // Return type + out param for extern decl
        let (extern_ret, extern_out_param) = match &m.ret {
            ReturnKind::Void => (quote! {}, None),
            ReturnKind::Value(vk) => {
                let ann = value_ret_annotation(vk);
                (ann, None)
            }
            ReturnKind::Result { ok_ty, .. } => {
                let out = ok_ty.as_ref().map(|vk| match vk {
                    ValueKind::Regular(ty) => quote! { result: *mut <#ty as ffier::FfiType>::CRepr, },
                    ValueKind::Slice(_) => quote! { result: *mut ffier::FfierBytes, },
                });
                (quote! { -> ffier::FfierError }, out)
            }
        };

        client_extern_decls.push(quote! {
            fn #ffi_name(#extern_handle_param #(#extern_params,)* #extern_out_param) #extern_ret;
        });

        // --- Build safe wrapper method ---

        // Receiver in safe wrapper signature
        let wrapper_receiver = if !m.has_receiver {
            None
        } else if m.is_by_value {
            Some(quote! { self, })
        } else if m.is_mut {
            Some(quote! { &mut self, })
        } else {
            Some(quote! { &self, })
        };

        // Wrapper params (original Rust types)
        let wrapper_params: Vec<_> = m.param_idents.iter()
            .zip(m.param_kinds.iter())
            .zip(m.param_orig_types.iter())
            .map(|((id, kind), orig_ty)| {
                match kind {
                    ParamKind::Slice(SliceKind::Str) => quote! { #id: &str },
                    ParamKind::Slice(SliceKind::Bytes) => quote! { #id: &[u8] },
                    ParamKind::Slice(SliceKind::Path) => quote! { #id: &std::path::Path },
                    ParamKind::StrSlice => quote! { #id: &[&str] },
                    ParamKind::HandleRef { .. } => {
                        // Use original type directly to preserve lifetime annotations
                        quote! { #id: #orig_ty }
                    }
                    ParamKind::DynDispatch(cfg) => {
                        let trait_name = format_ident!("Into{}Handle", cfg.c_name.trim_start_matches(&type_pfx));
                        quote! { #id: impl #trait_name }
                    }
                    ParamKind::Regular(_) => quote! { #id: #orig_ty },
                }
            })
            .collect();

        // Arg conversions (Rust value → FFI call arg)
        let wrapper_args: Vec<_> = m.param_idents.iter()
            .zip(m.param_kinds.iter())
            .zip(m.param_orig_types.iter())
            .map(|((id, kind), orig_ty)| {
                match kind {
                    ParamKind::Slice(SliceKind::Str) => quote! { ffier::FfierBytes::from_str(#id) },
                    ParamKind::Slice(SliceKind::Bytes) => quote! { ffier::FfierBytes::from_bytes(#id) },
                    ParamKind::Slice(SliceKind::Path) => quote! { ffier::FfierBytes::from_path(#id) },
                    ParamKind::StrSlice => {
                        quote! { __ffi_strs.as_ptr(), __ffi_strs.len() }
                    }
                    ParamKind::HandleRef { .. } => quote! { #id.0 },
                    ParamKind::DynDispatch(_) => {
                        quote! { #id.into_raw_handle() }
                    }
                    ParamKind::Regular(_) => {
                        // Use original type to preserve lifetime params
                        quote! { <#orig_ty as ffier::FfiType>::into_c(#id) }
                    }
                }
            })
            .collect();

        // Pre-bindings for StrSlice
        let wrapper_pre_bindings: Vec<_> = m.param_idents.iter()
            .zip(m.param_kinds.iter())
            .filter_map(|(id, kind)| {
                if matches!(kind, ParamKind::StrSlice) {
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
        let handle_arg = if !m.has_receiver {
            quote! {}
        } else if handle_is_indirect {
            quote! { &mut __handle, }
        } else {
            quote! { self.0, }
        };

        // Build the method body based on return kind and builder pattern
        let wrapper_body = if m.is_builder && m.is_by_value {
            // Builder pattern: by-value self → Self
            match &m.ret {
                ReturnKind::Void => {
                    // self → Self (non-Result builder)
                    quote! {
                        let mut __handle = {
                            let mut this = std::mem::ManuallyDrop::new(self);
                            this.0
                        };
                        #(#wrapper_pre_bindings)*
                        unsafe { #ffi_name(&mut __handle, #(#wrapper_args),*) };
                        Self(__handle #client_phantom_init)
                    }
                }
                ReturnKind::Result { ok_ty: None, err_ident, .. } => {
                    // self → Result<Self, E>
                    let err_ty = format_ident!("{err_ident}");
                    quote! {
                        let mut __handle = {
                            let mut this = std::mem::ManuallyDrop::new(self);
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
        } else if m.is_by_value && !m.has_receiver {
            // Static method
            build_client_static_or_instance_body(
                ffi_name, &m.ret, &handle_arg, &wrapper_args, &wrapper_pre_bindings,
                &m.ret_orig_type,
            )
        } else if m.is_by_value {
            // By-value self, non-builder (e.g. consume, or returning other type)
            let inner_body = build_client_static_or_instance_body(
                ffi_name, &m.ret, &quote! { __handle, }, &wrapper_args, &wrapper_pre_bindings,
                &m.ret_orig_type,
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
            build_client_static_or_instance_body(
                ffi_name, &m.ret, &handle_arg, &wrapper_args, &wrapper_pre_bindings,
                &m.ret_orig_type,
            )
        };

        // Doc comments
        let doc_attrs: Vec<_> = m.doc_lines.iter()
            .map(|line| quote! { #[doc = #line] })
            .collect();

        // Return type for safe wrapper signature
        let wrapper_ret_type = match &m.ret {
            ReturnKind::Void if m.is_builder => {
                // Builder returning Self
                quote! { -> Self }
            }
            ReturnKind::Void => quote! {},
            ReturnKind::Value(vk) => {
                client_value_ret_type(vk, &m.ret_orig_type, &struct_ident_for_client)
            }
            ReturnKind::Result { ok_ty, err_ident, .. } if m.is_builder => {
                let err_ty = format_ident!("{err_ident}");
                quote! { -> Result<Self, #err_ty> }
            }
            ReturnKind::Result { ok_ty, err_ident, .. } => {
                let err_ty = format_ident!("{err_ident}");
                match ok_ty {
                    None => quote! { -> Result<(), #err_ty> },
                    Some(vk) => {
                        let ok_ret = client_value_ok_type(vk, &m.ret_orig_type, &struct_ident_for_client);
                        quote! { -> Result<#ok_ret, #err_ty> }
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
        for (_id, kind) in m.param_idents.iter().zip(m.param_kinds.iter()) {
            if let ParamKind::DynDispatch(cfg) = kind {
                let trait_suffix = cfg.c_name.trim_start_matches(&type_pfx);
                let trait_name = format_ident!("Into{trait_suffix}Handle");
                let variant_impls: Vec<_> = cfg.variants.iter().map(|(_name, _)| {
                    let variant_ident = format_ident!("Vtable{trait_suffix}");
                    quote! {
                        impl #trait_name for #variant_ident {
                            fn into_raw_handle(self) -> *mut core::ffi::c_void {
                                let this = std::mem::ManuallyDrop::new(self);
                                this.0
                            }
                        }
                    }
                }).collect();
                client_dyn_traits.push(quote! {
                    pub trait #trait_name {
                        fn into_raw_handle(self) -> *mut core::ffi::c_void;
                    }
                    #(#variant_impls)*
                });
            }
        }
    }

    let output = quote! {
        #impl_block

        impl ffier::FfiHandle for #self_ty_static {
            const C_HANDLE_NAME: &str = #handle_c_name;
        }

        impl ffier::FfiType for #self_ty_static {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &str = #handle_c_name;
            fn into_c(self) -> *mut core::ffi::c_void {
                let tagged = ffier::FfierTaggedBox {
                    type_id: core::any::TypeId::of::<Self>(),
                    value: self,
                };
                Box::into_raw(Box::new(tagged)) as *mut core::ffi::c_void
            }
            fn from_c(repr: *mut core::ffi::c_void) -> Self {
                unsafe {
                    let tagged = Box::from_raw(
                        repr as *mut ffier::FfierTaggedBox<Self>
                    );
                    tagged.value
                }
            }
        }

        #(#warnings)*

        #[doc(hidden)]
        pub mod #helper_mod_name {
            #(#reexport_items)*
        }

        #[macro_export]
        macro_rules! #bridge_macro_name {
            ($struct_ty:ty) => {
                #(#ffi_fns)*

                pub fn #header_fn_name() -> ffier::HeaderSection {
                    let handle_typedef = #handle_typedef_expr .to_string();
                    let shared_lines: [String; #num_shared] = [
                        #(#shared_types_exprs .to_string()),*
                    ];
                    let shared_types = shared_lines.join("\n");
                    let decl_lines: [String; #num_decls] = [
                        #(#decl_exprs .to_string()),*
                    ];
                    let declarations = decl_lines.join("\n");
                    ffier::HeaderSection {
                        struct_name: #struct_name.to_string(),
                        handle_typedef,
                        shared_types,
                        declarations,
                    }
                }
            };
        }

        /// Client macro: generates safe Rust wrapper struct calling through C ABI.
        #[macro_export]
        macro_rules! #client_macro_name {
            () => {
                unsafe extern "C" {
                    #(#client_extern_decls)*
                }

                pub struct #struct_ident_for_client #client_struct_generics (
                    *mut core::ffi::c_void
                    #client_phantom
                );

                impl #client_struct_generics #struct_ident_for_client #client_struct_generics {
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

                impl #client_struct_generics std::fmt::Debug for #struct_ident_for_client #client_struct_generics {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.debug_tuple(#struct_name).field(&self.0).finish()
                    }
                }

                impl #client_struct_generics #struct_ident_for_client #client_struct_generics {
                    #(#client_methods)*
                }

                impl #client_struct_generics Drop for #struct_ident_for_client #client_struct_generics {
                    fn drop(&mut self) {
                        unsafe { #destroy_ident(self.0) }
                    }
                }

                #(#client_dyn_traits)*
            };
        }
    };

    output.into()
}

// ---------------------------------------------------------------------------
// Client codegen helpers
// ---------------------------------------------------------------------------

/// Build the method body for a non-builder static or instance method in the client wrapper.
fn build_client_static_or_instance_body(
    ffi_name: &syn::Ident,
    ret: &ReturnKind,
    handle_arg: &proc_macro2::TokenStream,
    wrapper_args: &[proc_macro2::TokenStream],
    wrapper_pre_bindings: &[proc_macro2::TokenStream],
    orig_ret: &ReturnType,
) -> proc_macro2::TokenStream {
    match ret {
        ReturnKind::Void => {
            quote! {
                #(#wrapper_pre_bindings)*
                unsafe { #ffi_name(#handle_arg #(#wrapper_args),*) }
            }
        }
        ReturnKind::Value(vk) => {
            let convert = client_value_from_ffi(vk, orig_ret);
            quote! {
                #(#wrapper_pre_bindings)*
                let __raw = unsafe { #ffi_name(#handle_arg #(#wrapper_args),*) };
                #convert
            }
        }
        ReturnKind::Result { ok_ty, err_ident, .. } => {
            let err_ty = format_ident!("{err_ident}");
            match ok_ty {
                None => {
                    quote! {
                        #(#wrapper_pre_bindings)*
                        let __err = unsafe { #ffi_name(#handle_arg #(#wrapper_args),*) };
                        if __err.code == 0 { Ok(()) } else { Err(#err_ty::from_ffi(__err)) }
                    }
                }
                Some(vk) => {
                    let (out_decl, out_ptr, ok_convert) = client_result_ok_from_ffi(vk, orig_ret);
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
fn client_value_from_ffi(vk: &ValueKind, orig_ret: &ReturnType) -> proc_macro2::TokenStream {
    match vk {
        ValueKind::Regular(_) => {
            if let ReturnType::Type(_, orig_ty) = orig_ret {
                if !is_primitive(orig_ty) {
                    let stripped = strip_lifetimes_from_generics(orig_ty);
                    // Use FfiType::from_c for non-lifetime types (works for handles + OwnedFd etc.)
                    // Use __from_raw for lifetime-parameterized types (always handles)
                    if has_lifetime_params(orig_ty) {
                        return quote! { #stripped::__from_raw(__raw) };
                    } else {
                        return quote! { <#stripped as ffier::FfiType>::from_c(__raw) };
                    }
                }
            }
            quote! { __raw }
        }
        // Use raw pointer conversion to avoid lifetime tie to the temporary FfierBytes
        ValueKind::Slice(SliceKind::Str) => quote! {
            unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(__raw.data, __raw.len)) }
        },
        ValueKind::Slice(SliceKind::Bytes) => quote! {
            unsafe { core::slice::from_raw_parts(__raw.data, __raw.len) }
        },
        ValueKind::Slice(SliceKind::Path) => quote! {
            unsafe { __raw.as_path() }
        },
    }
}

/// For Result<T, E> returns, build (out_decl, out_ptr_expr, ok_convert).
fn client_result_ok_from_ffi(vk: &ValueKind, orig_ret: &ReturnType) -> (proc_macro2::TokenStream, proc_macro2::TokenStream, proc_macro2::TokenStream) {
    match vk {
        ValueKind::Regular(_) => {
            if let ReturnType::Type(_, orig_ty) = orig_ret {
                if let Some((ok_ty, _)) = extract_result_types(orig_ty) {
                    if !is_primitive(&ok_ty) {
                        let stripped = strip_lifetimes_from_generics(&ok_ty);
                        let convert = if has_lifetime_params(&ok_ty) {
                            quote! { #stripped::__from_raw(unsafe { __out.assume_init() }) }
                        } else {
                            quote! { <#stripped as ffier::FfiType>::from_c(unsafe { __out.assume_init() }) }
                        };
                        return (
                            quote! { let mut __out = std::mem::MaybeUninit::uninit(); },
                            quote! { __out.as_mut_ptr() },
                            convert,
                        );
                    }
                }
            }
            (
                quote! { let mut __out = std::mem::MaybeUninit::uninit(); },
                quote! { __out.as_mut_ptr() },
                quote! { unsafe { __out.assume_init() } },
            )
        }
        ValueKind::Slice(SliceKind::Str) => (
            quote! { let mut __out = ffier::FfierBytes::EMPTY; },
            quote! { &mut __out },
            quote! { unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(__out.data, __out.len)) } },
        ),
        ValueKind::Slice(SliceKind::Bytes) => (
            quote! { let mut __out = ffier::FfierBytes::EMPTY; },
            quote! { &mut __out },
            quote! { unsafe { core::slice::from_raw_parts(__out.data, __out.len) } },
        ),
        ValueKind::Slice(SliceKind::Path) => (
            quote! { let mut __out = ffier::FfierBytes::EMPTY; },
            quote! { &mut __out },
            quote! { unsafe { __out.as_path() } },
        ),
    }
}

/// Check if a type has any lifetime parameters in its path.
fn has_lifetime_params(ty: &Type) -> bool {
    let Type::Path(tp) = ty else { return false };
    tp.path.segments.iter().any(|seg| {
        if let PathArguments::AngleBracketed(ab) = &seg.arguments {
            ab.args.iter().any(|arg| matches!(arg, GenericArgument::Lifetime(_)))
        } else {
            false
        }
    })
}

/// Remove all lifetime parameters from generic arguments (e.g. `View<'static>` → `View`).
fn strip_lifetimes_from_generics(ty: &Type) -> Type {
    struct Stripper;
    impl VisitMut for Stripper {
        fn visit_path_arguments_mut(&mut self, args: &mut PathArguments) {
            if let PathArguments::AngleBracketed(ab) = args {
                ab.args = ab.args.iter().filter(|arg| {
                    !matches!(arg, GenericArgument::Lifetime(_))
                }).cloned().collect();
                if ab.args.is_empty() {
                    *args = PathArguments::None;
                }
            }
            syn::visit_mut::visit_path_arguments_mut(self, args);
        }
    }
    let mut ty = ty.clone();
    Stripper.visit_type_mut(&mut ty);
    ty
}

/// Determine the Rust return type for the client wrapper from a ValueKind.
fn client_value_ret_type(
    vk: &ValueKind,
    orig_ret: &ReturnType,
    _struct_ident: &syn::Ident,
) -> proc_macro2::TokenStream {
    match vk {
        ValueKind::Slice(SliceKind::Str) => quote! { -> &str },
        ValueKind::Slice(SliceKind::Bytes) => quote! { -> &[u8] },
        ValueKind::Slice(SliceKind::Path) => quote! { -> &std::path::Path },
        ValueKind::Regular(_) => {
            if let ReturnType::Type(_, ty) = orig_ret {
                quote! { -> #ty }
            } else {
                quote! {}
            }
        }
    }
}

/// Determine the Ok type for Result returns in the client wrapper.
fn client_value_ok_type(
    vk: &ValueKind,
    orig_ret: &ReturnType,
    _struct_ident: &syn::Ident,
) -> proc_macro2::TokenStream {
    match vk {
        ValueKind::Slice(SliceKind::Str) => quote! { &str },
        ValueKind::Slice(SliceKind::Bytes) => quote! { &[u8] },
        ValueKind::Slice(SliceKind::Path) => quote! { &std::path::Path },
        ValueKind::Regular(_) => {
            if let ReturnType::Type(_, ty) = orig_ret {
                if let Some((ok_ty, _)) = extract_result_types(ty) {
                    quote! { #ok_ty }
                } else {
                    quote! { () }
                }
            } else {
                quote! { () }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_header_line(
    c_ret_expr: proc_macro2::TokenStream,
    ffi_name_str: &str,
    handle_type: Option<&String>,
    param_c_type_exprs: &[proc_macro2::TokenStream],
    param_name_strs: &[&String],
    out_param_c_type: Option<&proc_macro2::TokenStream>,
) -> proc_macro2::TokenStream {
    let out_snippet = out_param_c_type.map(|ct| {
        quote! {
            if need_comma { s.push_str(", "); }
            s.push_str(#ct);
            s.push_str("* result");
            need_comma = true;
        }
    });
    let handle_snippet = handle_type.map(|ht| {
        quote! {
            s.push_str(#ht);
            need_comma = true;
        }
    });
    quote! {{
        let c_type_names: &[&str] = &[#(#param_c_type_exprs),*];
        let param_names: &[&str] = &[#(#param_name_strs),*];
        let mut s = String::new();
        s.push_str(#c_ret_expr);
        s.push(' ');
        s.push_str(#ffi_name_str);
        s.push('(');
        let mut need_comma = false;
        #handle_snippet
        for (cty, name) in c_type_names.iter().zip(param_names.iter()) {
            if need_comma { s.push_str(", "); }
            s.push_str(cty);
            s.push(' ');
            s.push_str(name);
            need_comma = true;
        }
        #out_snippet
        if !need_comma { s.push_str("void"); }
        s.push_str(");");
        s
    }}
}

fn extract_result_types(ty: &Type) -> Option<(Type, Type)> {
    let Type::Path(tp) = ty else { return None };
    let last = tp.path.segments.last()?;
    if last.ident != "Result" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    let mut iter = args.args.iter();
    let GenericArgument::Type(ok_ty) = iter.next()? else {
        return None;
    };
    let GenericArgument::Type(err_ty) = iter.next()? else {
        return None;
    };
    Some((ok_ty.clone(), err_ty.clone()))
}

fn type_ident_name(ty: &Type) -> String {
    let Type::Path(tp) = ty else {
        return "Error".to_string();
    };
    tp.path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_else(|| "Error".to_string())
}

fn is_unit_type(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(t) if t.elems.is_empty())
}

/// Check if `ty` is the same type as `target` or a reference to it.
/// Matches: `Self`, `&Self`, `&mut Self` (after Self→concrete replacement).
fn is_self_return(ty: &Type, target: &Type) -> bool {
    let tgt = target.to_token_stream().to_string().replace(' ', "");
    let inner = match ty {
        Type::Reference(ref_ty) => ref_ty.elem.to_token_stream().to_string().replace(' ', ""),
        _ => ty.to_token_stream().to_string().replace(' ', ""),
    };
    inner == tgt
}

fn camel_to_snake(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn camel_to_upper_snake(s: &str) -> String {
    camel_to_snake(s).to_ascii_uppercase()
}

/// Replace all named lifetimes with `'static` so types can be used at the
/// FFI boundary (reexport modules, bridge macros) without free lifetime params.
fn erase_lifetimes(ty: &Type) -> Type {
    struct Eraser;
    impl VisitMut for Eraser {
        fn visit_lifetime_mut(&mut self, lt: &mut syn::Lifetime) {
            *lt = syn::Lifetime::new("'static", lt.apostrophe);
        }
    }
    let mut ty = ty.clone();
    Eraser.visit_type_mut(&mut ty);
    ty
}

/// Replace all occurrences of `Self` in a type with a concrete type.
fn replace_self_type(ty: &Type, replacement: &Type) -> Type {
    struct Replacer<'a>(&'a Type);
    impl VisitMut for Replacer<'_> {
        fn visit_type_mut(&mut self, ty: &mut Type) {
            if let Type::Path(tp) = ty {
                if tp.path.is_ident("Self") {
                    *ty = self.0.clone();
                    return;
                }
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }
    let mut ty = ty.clone();
    Replacer(replacement).visit_type_mut(&mut ty);
    ty
}

fn snake_to_pascal(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => {
                    let mut s = first.to_uppercase().to_string();
                    s.extend(c);
                    s
                }
                None => String::new(),
            }
        })
        .collect()
}

const PRIMITIVES: &[&str] = &[
    "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "isize", "usize", "bool", "f32", "f64",
];

fn type_tokens_for_macro(
    ty: &Type,
    reexport_types: &mut Vec<Type>,
    reexport_aliases: &mut Vec<syn::Ident>,
    counter: &mut u32,
    helper_mod: &syn::Ident,
) -> proc_macro2::TokenStream {
    if is_primitive(ty) {
        return quote! { #ty };
    }

    for (i, existing) in reexport_types.iter().enumerate() {
        if quote!(#existing).to_string() == quote!(#ty).to_string() {
            let alias = &reexport_aliases[i];
            return quote! { $crate::#helper_mod::#alias };
        }
    }

    let alias = format_ident!("_Type{counter}");
    *counter += 1;
    reexport_types.push(ty.clone());
    reexport_aliases.push(alias.clone());

    quote! { $crate::#helper_mod::#alias }
}

fn is_primitive(ty: &Type) -> bool {
    let Type::Path(tp) = ty else { return false };
    tp.path.segments.len() == 1
        && PRIMITIVES.contains(&tp.path.segments[0].ident.to_string().as_str())
}

// ---------------------------------------------------------------------------
// #[derive(FfiError)]
// ---------------------------------------------------------------------------

/// Derive `ffier::FfiError` for a unit enum.
///
/// Each variant must have `#[ffier(code = N)]` and optionally
/// `#[ffier(code = N, message = "...")]`. Without an explicit message,
/// the variant name is humanized (`DivisionByZero` → `"division by zero"`).
///
/// ```ignore
/// #[derive(ffier::FfiError)]
/// pub enum CalcError {
///     #[ffier(code = 1)]
///     DivisionByZero,
///     #[ffier(code = 2, message = "integer overflow")]
///     Overflow,
/// }
/// ```
#[proc_macro_derive(FfiError, attributes(ffier))]
pub fn derive_ffi_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let Data::Enum(data_enum) = &input.data else {
        return syn::Error::new_spanned(&input, "FfiError can only be derived for enums")
            .to_compile_error()
            .into();
    };

    let mut code_arms = Vec::new();
    let mut message_arms = Vec::new();
    let mut codes_entries = Vec::new();
    // For client error enum generation
    let mut client_variant_idents = Vec::new();
    let mut client_from_ffi_arms = Vec::new();
    let mut client_display_arms = Vec::new();

    for variant in &data_enum.variants {
        let var_ident = &variant.ident;

        let attrs = match parse_ffier_variant_attrs(&variant.attrs) {
            Ok(a) => a,
            Err(e) => return e.to_compile_error().into(),
        };

        let code = attrs.code;
        let message = attrs
            .message
            .unwrap_or_else(|| camel_to_human(&var_ident.to_string()));
        let upper_name = camel_to_upper_snake(&var_ident.to_string());

        code_arms.push(quote! { #name::#var_ident => #code });

        let msg_with_nul = format!("{message}\0");
        let msg_lit = proc_macro2::Literal::byte_string(msg_with_nul.as_bytes());
        message_arms.push(quote! {
            #code => unsafe {
                core::ffi::CStr::from_bytes_with_nul_unchecked(#msg_lit)
            }
        });

        codes_entries.push(quote! { (#upper_name, #code) });

        // Client codegen
        client_variant_idents.push(var_ident.clone());
        client_from_ffi_arms.push(quote! { #code => Self::#var_ident });
        client_display_arms.push(quote! { Self::#var_ident => write!(f, #message) });
    }

    let unknown_msg = format!(
        "unknown {} error\0",
        camel_to_snake(&name.to_string()).replace('_', " ")
    );
    let unknown_lit = proc_macro2::Literal::byte_string(unknown_msg.as_bytes());

    // Generate a bridge macro for the error type's FFI helpers.
    // The cdylib invokes this ONCE with the prefix to emit the no_mangle functions.
    let name_str = name.to_string();
    let err_snake = camel_to_snake(&name_str);
    let err_snake_ident = format_ident!("{err_snake}");
    let err_upper = camel_to_upper_snake(&name_str);
    let err_upper_ident = format_ident!("{err_upper}");
    let err_snake_msg_suffix = format!("_{err_snake}_message");
    let err_snake_free_suffix = format!("_{err_snake}_free");
    let bridge_macro_name = format_ident!("{err_snake}_error_ffier");
    let client_error_macro_name = format_ident!("{err_snake}_ffi_client");

    let output = quote! {
        impl ffier::FfiError for #name {
            fn code(&self) -> u64 {
                match self {
                    #(#code_arms,)*
                }
            }

            fn static_message(code: u64) -> &'static core::ffi::CStr {
                match code {
                    #(#message_arms,)*
                    _ => unsafe {
                        core::ffi::CStr::from_bytes_with_nul_unchecked(#unknown_lit)
                    },
                }
            }

            fn codes() -> &'static [(&'static str, u64)] {
                &[#(#codes_entries),*]
            }
        }

        /// Bridge macro for error type FFI helpers. Invoke once in the cdylib
        /// with the prefix: `mycrate::my_error_ffier!("prefix");`
        #[macro_export]
        macro_rules! #bridge_macro_name {

            ($fn_pfx:literal) => {
                ffier::paste::paste! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn [<$fn_pfx _ #err_snake_ident _message>](
                        err: *const ffier::FfierError,
                    ) -> *const core::ffi::c_char {
                        let err = unsafe { &*err };
                        let ptr = err.msg_ptr();
                        if !ptr.is_null() { return ptr; }
                        <$crate::#name as ffier::FfiError>::static_message(err.code).as_ptr()
                    }

                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn [<$fn_pfx _ #err_snake_ident _free>](
                        err: *mut ffier::FfierError,
                    ) {
                        unsafe { (*err).free() };
                    }

                    pub fn [<$fn_pfx _ #err_snake_ident __header>]() -> ffier::HeaderSection {
                        let err_c_name = concat!(stringify!([<$fn_pfx:camel>]), #name_str);
                        let message_fn_str = concat!($fn_pfx, #err_snake_msg_suffix);
                        let free_fn_str = concat!($fn_pfx, #err_snake_free_suffix);
                        let full_upper_pfx = stringify!([<$fn_pfx:upper _ #err_upper_ident>]);

                        let mut decls = String::new();
                        decls.push_str("typedef struct {\n");
                        decls.push_str("    uint64_t code;\n");
                        decls.push_str("    char* _msg; /* private */\n");
                        decls.push_str(&format!("}} {};\n\n", err_c_name));

                        for (variant_name, val) in <$crate::#name as ffier::FfiError>::codes() {
                            decls.push_str(&format!(
                                "#define {}_{} {}\n",
                                full_upper_pfx, variant_name, val
                            ));
                        }
                        decls.push_str(&format!(
                            "\nconst char* {}(const {}* err);\n",
                            message_fn_str, err_c_name
                        ));
                        decls.push_str(&format!(
                            "void {}({}* err);\n",
                            free_fn_str, err_c_name
                        ));

                        ffier::HeaderSection {
                            struct_name: #name_str.to_string(),
                            handle_typedef: String::new(),
                            shared_types: String::new(),
                            declarations: decls,
                        }
                    }
                }
            };
        }

        /// Client macro: generates a matching error enum for FFI client bindings.
        #[macro_export]
        macro_rules! #client_error_macro_name {
            () => {
                #[derive(Debug, Clone, Copy, PartialEq, Eq)]
                pub enum #name {
                    #(#client_variant_idents,)*
                }

                impl #name {
                    pub fn from_ffi(mut err: ffier::FfierError) -> Self {
                        let code = err.code;
                        unsafe { err.free() };
                        match code {
                            #(#client_from_ffi_arms,)*
                            other => panic!(concat!("unknown ", #name_str, " error code {}"), other),
                        }
                    }
                }

                impl std::fmt::Display for #name {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        match self {
                            #(#client_display_arms,)*
                        }
                    }
                }

                impl std::error::Error for #name {}
            };
        }
    };

    output.into()
}

struct FfierVariantAttrs {
    code: u64,
    message: Option<String>,
}

fn parse_ffier_variant_attrs(attrs: &[syn::Attribute]) -> syn::Result<FfierVariantAttrs> {
    for attr in attrs {
        if !attr.path().is_ident("ffier") {
            continue;
        }

        let mut code = None;
        let mut message = None;

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("code") {
                let value = meta.value()?;
                let lit: syn::LitInt = value.parse()?;
                code = Some(lit.base10_parse::<u64>()?);
                Ok(())
            } else if meta.path.is_ident("message") {
                let value = meta.value()?;
                let lit: LitStr = value.parse()?;
                message = Some(lit.value());
                Ok(())
            } else {
                Err(meta.error("expected `code` or `message`"))
            }
        })?;

        let code = code
            .ok_or_else(|| syn::Error::new_spanned(attr, "missing `code` in #[ffier(code = N)]"))?;

        return Ok(FfierVariantAttrs { code, message });
    }

    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        "missing #[ffier(code = N)] attribute on variant",
    ))
}

/// Parse `#[ffier(dyn_param(param_name, "CName", [Type1, Type2]))]` from method attributes.
///
/// Returns `Vec<(param_name, full_c_type_name, [(ident_name, type_tokens)])>`.
fn parse_dyn_param_attrs(
    attrs: &[syn::Attribute],
    reexport_types: &mut Vec<Type>,
    reexport_aliases: &mut Vec<syn::Ident>,
    alias_counter: &mut u32,
    helper_mod: &syn::Ident,
    type_pfx: &str,
) -> Vec<(String, String, Vec<(String, proc_macro2::TokenStream)>)> {
    let mut result = Vec::new();

    for attr in attrs {
        if !attr.path().is_ident("ffier") {
            continue;
        }

        // Parse: ffier(dyn_param(dev, "Device", [NetDevice, BlockDevice]))
        let _ = attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("dyn_param") {
                return Ok(());
            }

            let content;
            syn::parenthesized!(content in meta.input);

            // param name
            let param_ident: syn::Ident = content.parse()?;
            content.parse::<Token![,]>()?;

            // C type name suffix
            let c_name_lit: LitStr = content.parse()?;
            content.parse::<Token![,]>()?;

            // [Type1, Type2, ...]
            let types_content;
            syn::bracketed!(types_content in content);
            let variant_types: syn::punctuated::Punctuated<Type, Token![,]> =
                types_content.parse_terminated(Type::parse, Token![,])?;

            let c_name = format!("{type_pfx}{}", c_name_lit.value());
            let variants: Vec<_> = variant_types
                .iter()
                .map(|ty| {
                    let ident_name = type_ident_name(ty);
                    let erased = erase_lifetimes(ty);
                    let tokens = type_tokens_for_macro(
                        &erased,
                        reexport_types,
                        reexport_aliases,
                        alias_counter,
                        helper_mod,
                    );
                    (ident_name, tokens)
                })
                .collect();

            result.push((param_ident.to_string(), c_name, variants));
            Ok(())
        });
    }

    result
}

/// `DivisionByZero` → `"division by zero"`
fn camel_to_human(s: &str) -> String {
    camel_to_snake(s).replace('_', " ")
}

/// Extract `/// doc` comments from attributes.
fn extract_doc_comments(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            let syn::Meta::NameValue(nv) = &attr.meta else {
                return None;
            };
            let syn::Expr::Lit(lit) = &nv.value else {
                return None;
            };
            let syn::Lit::Str(s) = &lit.lit else {
                return None;
            };
            Some(s.value())
        })
        .collect()
}

/// Parsed doc comment sections.
struct DocSections {
    /// Lines before any `# Arguments` / `# Returns` heading.
    body: Vec<String>,
    /// `param_name` → description (from `# Arguments` section).
    param_docs: Vec<(String, String)>,
    /// Text from `# Returns` section.
    returns_doc: Option<String>,
}

/// Parse Rust doc lines into body, `# Arguments`, and `# Returns` sections.
///
/// Recognizes:
/// - `# Arguments` / `# Parameters` heading
///   - `* \`name\` - description` or `- \`name\` - description` entries
/// - `# Returns` / `# Return value` heading
///   - All following lines until the next `#` heading
fn parse_doc_sections(doc_lines: &[String]) -> DocSections {
    let mut body = Vec::new();
    let mut param_docs = Vec::new();
    let mut returns_doc = None;

    enum Section {
        Body,
        Arguments,
        Returns,
    }
    let mut section = Section::Body;
    let mut returns_lines: Vec<String> = Vec::new();

    for raw in doc_lines {
        let line = raw.strip_prefix(' ').unwrap_or(raw);

        // Detect heading transitions
        let lower = line.trim().to_lowercase();
        if lower == "# arguments" || lower == "# parameters" {
            section = Section::Arguments;
            continue;
        }
        if lower.starts_with("# return") {
            section = Section::Returns;
            continue;
        }
        // Any other `#` heading ends the current special section
        if line.trim().starts_with("# ") {
            // Flush returns
            if !returns_lines.is_empty() {
                returns_doc = Some(returns_lines.join(" ").trim().to_string());
                returns_lines.clear();
            }
            section = Section::Body;
            // Fall through to add this line to body
        }

        match section {
            Section::Body => body.push(raw.clone()),
            Section::Arguments => {
                // Parse `* \`name\` - description` or `- \`name\` - description`
                let trimmed = line.trim();
                let after_bullet = trimmed
                    .strip_prefix("* ")
                    .or_else(|| trimmed.strip_prefix("- "));
                if let Some(rest) = after_bullet {
                    if let Some((name, desc)) = parse_param_entry(rest) {
                        param_docs.push((name, desc));
                    }
                }
                // Ignore non-matching lines in the section (blank lines, etc.)
            }
            Section::Returns => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    returns_lines.push(trimmed.to_string());
                }
            }
        }
    }

    // Flush any remaining returns lines
    if !returns_lines.is_empty() {
        returns_doc = Some(returns_lines.join(" ").trim().to_string());
    }

    // Trim trailing blank lines from body
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }

    DocSections {
        body,
        param_docs,
        returns_doc,
    }
}

/// Parse `` `name` - description `` or `` `name` description ``.
fn parse_param_entry(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let rest = s.strip_prefix('`')?;
    let end = rest.find('`')?;
    let name = rest[..end].to_string();
    let after = rest[end + 1..].trim();
    let desc = after.strip_prefix('-').unwrap_or(after).trim().to_string();
    Some((name, desc))
}

/// Build a Doxygen comment block string from doc lines and method metadata.
fn build_doxygen_comment(
    doc_lines: &[String],
    param_names: &[String],
    has_out_param: bool,
    err_c_name: Option<&str>,
) -> Option<String> {
    if doc_lines.is_empty() {
        return None;
    }

    let sections = parse_doc_sections(doc_lines);

    if sections.body.is_empty() && sections.param_docs.is_empty() && sections.returns_doc.is_none()
    {
        return None;
    }

    let mut out = String::from("/**\n");

    // Body text
    for line in &sections.body {
        let trimmed = line.strip_prefix(' ').unwrap_or(line);
        if trimmed.is_empty() {
            out.push_str(" *\n");
        } else {
            out.push_str(&format!(" * {trimmed}\n"));
        }
    }

    // @param entries
    let has_params = !param_names.is_empty() || has_out_param;
    let has_return = err_c_name.is_some() || sections.returns_doc.is_some();
    if has_params || has_return {
        out.push_str(" *\n");
    }

    for name in param_names {
        let desc = sections
            .param_docs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d.as_str())
            .unwrap_or("");
        if desc.is_empty() {
            out.push_str(&format!(" * @param {name}\n"));
        } else {
            out.push_str(&format!(" * @param {name} {desc}\n"));
        }
    }

    if has_out_param {
        // For Result methods, Rust's `# Returns` describes the Ok value,
        // which maps to the C `out` parameter (not the C return).
        if let Some(ref doc) = sections.returns_doc {
            out.push_str(&format!(" * @param[out] result {doc}\n"));
        } else {
            out.push_str(" * @param[out] result\n");
        }
    }

    // @return
    if let Some(err_name) = err_c_name {
        out.push_str(&format!(
            " * @return {err_name} with code 0 on success, error code on failure.\n"
        ));
    } else if let Some(ref doc) = sections.returns_doc {
        out.push_str(&format!(" * @return {doc}\n"));
    }

    out.push_str(" */");
    Some(out)
}

// ===========================================================================
// #[ffier::implementable] — C users can implement a Rust trait via vtable
// ===========================================================================

struct ImplementableArgs {
    prefix: Option<String>,
    supers: Vec<SupertraitBlock>,
}

struct SupertraitBlock {
    trait_name: syn::Ident,
    methods: Vec<syn::TraitItemFn>,
}

impl Parse for ImplementableArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut prefix = None;
        let mut supers = Vec::new();

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;

            if ident == "prefix" {
                input.parse::<Token![=]>()?;
                let lit: LitStr = input.parse()?;
                prefix = Some(lit.value());
            } else if ident == "supers" {
                let content;
                syn::parenthesized!(content in input);
                while !content.is_empty() {
                    let trait_name: syn::Ident = content.parse()?;
                    let methods_content;
                    syn::braced!(methods_content in content);
                    let mut methods = Vec::new();
                    while !methods_content.is_empty() {
                        methods.push(methods_content.parse::<syn::TraitItemFn>()?);
                    }
                    supers.push(SupertraitBlock {
                        trait_name,
                        methods,
                    });
                    // optional comma between supertrait blocks
                    let _ = content.parse::<Token![,]>();
                }
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    "expected `prefix` or `supers`",
                ));
            }
            let _ = input.parse::<Token![,]>();
        }

        Ok(Self { prefix, supers })
    }
}

/// A method signature extracted from a trait, classified for vtable generation.
struct VtableMethod {
    name: syn::Ident,
    /// The trait this method belongs to (None = the annotated trait itself)
    trait_name: Option<syn::Ident>,
    /// Parameters (excluding &self), as (ident, ffi_type) pairs
    params: Vec<(syn::Ident, VtableParamType)>,
    /// Return type
    ret: VtableRetType,
}

enum VtableParamType {
    Primitive(Type),
    Str,
    Bytes,
    Path,
    Handle(Type),
}

enum VtableRetType {
    Void,
    Primitive(Type),
    Str,
    Bytes,
    Path,
    Handle(Type),
}

fn classify_vtable_param(ty: &Type) -> VtableParamType {
    if let Some(sk) = classify_ref_type(ty) {
        return match sk {
            SliceKind::Str => VtableParamType::Str,
            SliceKind::Bytes => VtableParamType::Bytes,
            SliceKind::Path => VtableParamType::Path,
        };
    }
    if let Type::Reference(r) = ty {
        return VtableParamType::Handle(erase_lifetimes(&r.elem));
    }
    VtableParamType::Primitive(erase_lifetimes(ty))
}

fn classify_vtable_ret(ty: &Type) -> VtableRetType {
    if let Some(sk) = classify_ref_type(ty) {
        return match sk {
            SliceKind::Str => VtableRetType::Str,
            SliceKind::Bytes => VtableRetType::Bytes,
            SliceKind::Path => VtableRetType::Path,
        };
    }
    if let Type::Reference(r) = ty {
        return VtableRetType::Handle(erase_lifetimes(&r.elem));
    }
    VtableRetType::Primitive(erase_lifetimes(ty))
}

fn extract_vtable_methods(trait_item: &ItemTrait, supers: &[SupertraitBlock]) -> Vec<VtableMethod> {
    let mut methods = Vec::new();

    // Methods from the trait itself
    for item in &trait_item.items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        if let Some(vm) = parse_trait_method_sig(&method.sig, None) {
            methods.push(vm);
        }
    }

    // Methods from supertrait blocks
    for sup in supers {
        for method in &sup.methods {
            if let Some(vm) = parse_trait_method_sig(&method.sig, Some(sup.trait_name.clone())) {
                methods.push(vm);
            }
        }
    }

    methods
}

fn parse_trait_method_sig(
    sig: &syn::Signature,
    trait_name: Option<syn::Ident>,
) -> Option<VtableMethod> {
    // Must have &self receiver
    let first = sig.inputs.first()?;
    if !matches!(first, FnArg::Receiver(_)) {
        return None;
    }

    let params: Vec<_> = sig
        .inputs
        .iter()
        .skip(1)
        .filter_map(|arg| {
            let FnArg::Typed(pt) = arg else { return None };
            let Pat::Ident(pi) = &*pt.pat else {
                return None;
            };
            Some((pi.ident.clone(), classify_vtable_param(&pt.ty)))
        })
        .collect();

    let ret = match &sig.output {
        ReturnType::Default => VtableRetType::Void,
        ReturnType::Type(_, ty) => classify_vtable_ret(ty),
    };

    Some(VtableMethod {
        name: sig.ident.clone(),
        trait_name,
        params,
        ret,
    })
}

/// Generate the C function pointer type for a vtable method parameter
fn vtable_ffi_param_type(vpt: &VtableParamType) -> proc_macro2::TokenStream {
    match vpt {
        VtableParamType::Primitive(ty) => quote! { <#ty as ffier::FfiType>::CRepr },
        VtableParamType::Str | VtableParamType::Bytes | VtableParamType::Path => {
            quote! { ffier::FfierBytes }
        }
        VtableParamType::Handle(_) => quote! { *mut core::ffi::c_void },
    }
}

fn vtable_ffi_ret_type(vrt: &VtableRetType) -> proc_macro2::TokenStream {
    match vrt {
        VtableRetType::Void => quote! {},
        VtableRetType::Primitive(ty) => quote! { -> <#ty as ffier::FfiType>::CRepr },
        VtableRetType::Str | VtableRetType::Bytes | VtableRetType::Path => {
            quote! { -> ffier::FfierBytes }
        }
        VtableRetType::Handle(_) => quote! { -> *mut core::ffi::c_void },
    }
}

/// Generate Rust code to convert a vtable call result back to the trait return type
fn vtable_result_conversion(
    vrt: &VtableRetType,
    expr: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    match vrt {
        VtableRetType::Void => expr,
        VtableRetType::Primitive(ty) => quote! { <#ty as ffier::FfiType>::from_c(#expr) },
        VtableRetType::Str => quote! { unsafe {
            let __b = #expr;
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(__b.data, __b.len))
        }},
        VtableRetType::Bytes => quote! { unsafe {
            let __b = #expr;
            core::slice::from_raw_parts(__b.data, __b.len)
        }},
        VtableRetType::Path => quote! { unsafe { (#expr).as_path() } },
        VtableRetType::Handle(ty) => quote! { <#ty as ffier::FfiType>::from_c(#expr) },
    }
}

/// Generate Rust code to convert a Rust param value to the vtable call arg
fn vtable_arg_conversion(vpt: &VtableParamType, ident: &syn::Ident) -> proc_macro2::TokenStream {
    match vpt {
        VtableParamType::Primitive(ty) => quote! { <#ty as ffier::FfiType>::into_c(#ident) },
        VtableParamType::Str => quote! { ffier::FfierBytes::from_str(#ident) },
        VtableParamType::Bytes => quote! { ffier::FfierBytes::from_bytes(#ident) },
        VtableParamType::Path => quote! { ffier::FfierBytes::from_path(#ident) },
        VtableParamType::Handle(ty) => quote! { <#ty as ffier::FfiType>::into_c(#ident) },
    }
}

#[proc_macro_attribute]
pub fn implementable(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ImplementableArgs);
    let trait_item = parse_macro_input!(item as ItemTrait);
    let original_trait = trait_item.clone();

    let trait_name = &trait_item.ident;
    let trait_name_str = trait_name.to_string();
    let trait_snake = camel_to_snake(&trait_name_str);
    let fn_pfx = args
        .prefix
        .as_ref()
        .map(|p| format!("{p}_"))
        .unwrap_or_default();
    let type_pfx = args
        .prefix
        .as_ref()
        .map(|p| snake_to_pascal(p))
        .unwrap_or_default();

    let vtable_c_name = format!("{type_pfx}{trait_name_str}Vtable");
    let wrapper_name = format_ident!("Vtable{trait_name_str}");
    let wrapper_c_handle = format!("{type_pfx}Vtable{trait_name_str}");
    let vtable_struct_name = format_ident!("{type_pfx}{trait_name_str}Vtable");
    let constructor_name = format_ident!("{fn_pfx}{trait_snake}_from_vtable");
    let constructor_name_str = constructor_name.to_string();

    // Extract all methods (trait + supertraits)
    let vtable_methods = extract_vtable_methods(&trait_item, &args.supers);

    // --- Generate vtable struct fields ---
    let vtable_fields: Vec<_> = vtable_methods
        .iter()
        .map(|m| {
            let name = &m.name;
            let params: Vec<_> = m
                .params
                .iter()
                .map(|(_, vpt)| vtable_ffi_param_type(vpt))
                .collect();
            let ret = vtable_ffi_ret_type(&m.ret);
            quote! {
                pub #name: unsafe extern "C" fn(*mut core::ffi::c_void, #(#params),*) #ret
            }
        })
        .collect();

    // --- Generate trait impl method bodies (call through vtable) ---
    // Group methods by trait
    let mut own_methods = Vec::new();
    let mut super_methods: std::collections::HashMap<String, Vec<&VtableMethod>> =
        std::collections::HashMap::new();

    for m in &vtable_methods {
        match &m.trait_name {
            None => own_methods.push(m),
            Some(tn) => super_methods.entry(tn.to_string()).or_default().push(m),
        }
    }

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
    // Also erase lifetimes in method signatures
    let trait_item_erased = {
        let mut t = trait_item.clone();
        struct Eraser;
        impl VisitMut for Eraser {
            fn visit_lifetime_mut(&mut self, lt: &mut syn::Lifetime) {
                *lt = syn::Lifetime::new("'static", lt.apostrophe);
            }
        }
        Eraser.visit_item_trait_mut(&mut t);
        t
    };

    let own_method_impls: Vec<_> = trait_item_erased
        .items
        .iter()
        .filter_map(|item| {
            let TraitItem::Fn(method) = item else {
                return None;
            };
            let name = &method.sig.ident;
            let vm = vtable_methods.iter().find(|v| v.name == *name)?;

            let sig = &method.sig;
            let vtable_args: Vec<_> = vm
                .params
                .iter()
                .map(|(id, vpt)| vtable_arg_conversion(vpt, id))
                .collect();
            let call = quote! {
                unsafe { ((*self.vtable).#name)(self.user_data, #(#vtable_args),*) }
            };
            let body = vtable_result_conversion(&vm.ret, call);

            Some(quote! {
                #sig { #body }
            })
        })
        .collect();

    // Supertrait impls
    let super_impls: Vec<_> = args
        .supers
        .iter()
        .map(|sup| {
            let tn = &sup.trait_name;
            let method_impls: Vec<_> = sup
                .methods
                .iter()
                .filter_map(|method| {
                    let name = &method.sig.ident;
                    let vm = vtable_methods.iter().find(|v| v.name == *name)?;

                    let sig = &method.sig;
                    let vtable_args: Vec<_> = vm
                        .params
                        .iter()
                        .map(|(id, vpt)| vtable_arg_conversion(vpt, id))
                        .collect();
                    let call = quote! {
                        unsafe { ((*self.vtable).#name)(self.user_data, #(#vtable_args),*) }
                    };
                    let body = vtable_result_conversion(&vm.ret, call);

                    Some(quote! { #sig { #body } })
                })
                .collect();

            quote! {
                impl #tn for #wrapper_name {
                    #(#method_impls)*
                }
            }
        })
        .collect();

    // --- Header generation (via bridge macro) ---
    let bridge_macro_name = format_ident!("vtable_{trait_snake}_ffier");
    let client_vtable_macro_name = format_ident!("vtable_{trait_snake}_ffi_client");
    let header_fn_name = format_ident!("{fn_pfx}vtable_{trait_snake}__header");
    let vtable_section_name = format!("Vtable{trait_name_str}");

    // Build header lines for vtable struct
    let mut header_lines: Vec<proc_macro2::TokenStream> = Vec::new();

    header_lines.push(quote! { concat!("typedef struct {") });

    // For each method, generate a C function pointer line
    for m in &vtable_methods {
        let name_str = m.name.to_string();
        // Build C return type and params at runtime
        let param_c_types: Vec<_> = m
            .params
            .iter()
            .map(|(id, vpt)| {
                let id_str = id.to_string();
                let type_expr = match vpt {
                    VtableParamType::Primitive(ty) => {
                        quote! { <#ty as ffier::FfiType>::C_TYPE_NAME }
                    }
                    VtableParamType::Str => {
                        let n = format!("{type_pfx}Str");
                        quote! { #n }
                    }
                    VtableParamType::Bytes => {
                        let n = format!("{type_pfx}Bytes");
                        quote! { #n }
                    }
                    VtableParamType::Path => {
                        let n = format!("{type_pfx}Path");
                        quote! { #n }
                    }
                    VtableParamType::Handle(_) => quote! { "void*" },
                };
                (id_str, type_expr)
            })
            .collect();

        let ret_c_expr = match &m.ret {
            VtableRetType::Void => quote! { "void" },
            VtableRetType::Primitive(ty) => quote! { <#ty as ffier::FfiType>::C_TYPE_NAME },
            VtableRetType::Str => {
                let n = format!("{type_pfx}Str");
                quote! { #n }
            }
            VtableRetType::Bytes => {
                let n = format!("{type_pfx}Bytes");
                quote! { #n }
            }
            VtableRetType::Path => {
                let n = format!("{type_pfx}Path");
                quote! { #n }
            }
            VtableRetType::Handle(_) => quote! { "void*" },
        };

        let param_id_strs: Vec<_> = param_c_types.iter().map(|(id, _)| id.clone()).collect();
        let param_type_exprs: Vec<_> = param_c_types.iter().map(|(_, te)| te.clone()).collect();

        header_lines.push(quote! {{
            let mut s = String::from("    ");
            s.push_str(#ret_c_expr);
            s.push_str(" (*");
            s.push_str(#name_str);
            s.push_str(")(void* self_data");
            let param_types: &[&str] = &[#(#param_type_exprs),*];
            let param_names: &[&str] = &[#(#param_id_strs),*];
            for (t, n) in param_types.iter().zip(param_names.iter()) {
                s.push_str(", ");
                s.push_str(t);
                s.push(' ');
                s.push_str(n);
            }
            s.push_str(");");
            s
        }});
    }

    // drop function pointer
    header_lines.push(quote! { "    void (*drop)(void* self_data);" });
    header_lines.push(quote! { concat!("} ", #vtable_c_name, ";") });
    header_lines.push(quote! { "" });
    header_lines.push(quote! {
        concat!("void* ", #constructor_name_str, "(void* user_data, const ", #vtable_c_name, "* vtable);")
    });

    let num_header_lines = header_lines.len();

    let output = quote! {
        #original_trait

        #[repr(C)]
        pub struct #vtable_struct_name {
            #(#vtable_fields,)*
            pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
        }

        pub struct #wrapper_name {
            pub user_data: *mut core::ffi::c_void,
            pub vtable: *const #vtable_struct_name,
        }

        impl #trait_name #trait_ty_generics for #wrapper_name {
            #(#own_method_impls)*
        }

        #(#super_impls)*

        impl Drop for #wrapper_name {
            fn drop(&mut self) {
                if let Some(drop_fn) = unsafe { (*self.vtable).drop } {
                    unsafe { drop_fn(self.user_data) };
                }
            }
        }

        impl ffier::FfiHandle for #wrapper_name {
            const C_HANDLE_NAME: &str = #wrapper_c_handle;
        }

        impl ffier::FfiType for #wrapper_name {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &str = #wrapper_c_handle;
            fn into_c(self) -> *mut core::ffi::c_void {
                let tagged = ffier::FfierTaggedBox {
                    type_id: core::any::TypeId::of::<Self>(),
                    value: self,
                };
                Box::into_raw(Box::new(tagged)) as *mut core::ffi::c_void
            }
            fn from_c(repr: *mut core::ffi::c_void) -> Self {
                unsafe {
                    let tagged = Box::from_raw(
                        repr as *mut ffier::FfierTaggedBox<Self>
                    );
                    tagged.value
                }
            }
        }

        #[macro_export]
        macro_rules! #bridge_macro_name {
            () => {
                #[unsafe(no_mangle)]
                pub extern "C" fn #constructor_name(
                    user_data: *mut core::ffi::c_void,
                    vtable: *const $crate::#vtable_struct_name,
                ) -> *mut core::ffi::c_void {
                    let wrapper = $crate::#wrapper_name {
                        user_data,
                        vtable,
                    };
                    <$crate::#wrapper_name as ffier::FfiType>::into_c(wrapper)
                }

                pub fn #header_fn_name() -> ffier::HeaderSection {
                    let decl_lines: [String; #num_header_lines] = [
                        #(#header_lines .to_string()),*
                    ];
                    let declarations = decl_lines.join("\n");
                    ffier::HeaderSection {
                        struct_name: #vtable_section_name.to_string(),
                        handle_typedef: String::new(),
                        shared_types: String::new(),
                        declarations,
                    }
                }
            };
        }

        /// Client macro: generates vtable struct + handle wrapper for FFI client bindings.
        #[macro_export]
        macro_rules! #client_vtable_macro_name {
            () => {
                #[repr(C)]
                pub struct #vtable_struct_name {
                    #(#vtable_fields,)*
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
                        // The vtable wrapper's destructor is handled by the library side
                        // when the handle is destroyed. We don't have a separate destroy
                        // function for vtable handles — they are consumed by dyn_param methods.
                    }
                }
            };
        }
    };

    output.into()
}
