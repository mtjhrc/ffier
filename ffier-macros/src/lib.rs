use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, FnArg, GenericArgument, ImplItem, ItemImpl, LitStr, Pat, PathArguments,
    ReturnType, Token, Type, parse::Parse, parse_macro_input,
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
        err_ty: proc_macro2::TokenStream,
        err_ident: String,
    },
}

struct MethodInfo {
    method_name: syn::Ident,
    ffi_name: syn::Ident,
    ffi_name_str: String,
    is_mut: bool,
    param_idents: Vec<syn::Ident>,
    param_name_strs: Vec<String>,
    param_kinds: Vec<ParamKind>,
    ret: ReturnKind,
    doc_lines: Vec<String>,
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
    }
}

fn param_conversion(id: &syn::Ident, kind: &ParamKind) -> proc_macro2::TokenStream {
    match kind {
        ParamKind::Regular(ty) => quote! { <#ty as ffier::FfiType>::from_c(#id) },
        ParamKind::Slice(SliceKind::Str) => quote! { unsafe { #id.as_str_unchecked() } },
        ParamKind::Slice(SliceKind::Bytes) => quote! { unsafe { #id.as_bytes() } },
        ParamKind::Slice(SliceKind::Path) => quote! { unsafe { #id.as_path() } },
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
    }
}

fn value_ret_annotation(kind: &ValueKind) -> proc_macro2::TokenStream {
    match kind {
        ValueKind::Regular(ty) => quote! { -> <#ty as ffier::FfiType>::CRepr },
        ValueKind::Slice(_) => quote! { -> ffier::FfierBytes },
    }
}

fn value_into_c(kind: &ValueKind) -> proc_macro2::TokenStream {
    match kind {
        ValueKind::Regular(ty) => quote! { <#ty as ffier::FfiType>::into_c(result) },
        ValueKind::Slice(SliceKind::Str) => quote! { ffier::FfierBytes::from_str(result) },
        ValueKind::Slice(SliceKind::Bytes) => quote! { ffier::FfierBytes::from_bytes(result) },
        ValueKind::Slice(SliceKind::Path) => quote! { ffier::FfierBytes::from_path(result) },
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
    let impl_block = input.clone();

    let Type::Path(ref struct_path) = *input.self_ty else {
        return syn::Error::new_spanned(&input.self_ty, "expected a named struct type")
            .to_compile_error()
            .into();
    };
    let struct_ident = struct_path.path.get_ident().expect("expected simple struct name");
    let struct_name = struct_ident.to_string();
    let struct_lower = struct_name.to_lowercase();

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

    let handle_c_name = format!("{type_pfx}{struct_name}Handle");
    let bytes_c_name = format!("{type_pfx}Bytes");
    let str_c_name = format!("{type_pfx}Str");
    let path_c_name = format!("{type_pfx}Path");
    let str_macro_name = format!("{upper_pfx}STR");

    let trait_path = input.trait_.as_ref().map(|(_, path, _)| path);

    let struct_upper = camel_to_upper_snake(&struct_name);
    let header_guard = format!("{upper_pfx}{struct_upper}_H");

    let mut ffi_fns = Vec::new();
    let mut header_exprs: Vec<proc_macro2::TokenStream> = Vec::new();

    header_exprs
        .push(quote! { concat!("/* Auto-generated by ffier for ", #struct_name, " */") });
    header_exprs.push(quote! { concat!("#ifndef ", #header_guard) });
    header_exprs.push(quote! { concat!("#define ", #header_guard) });
    header_exprs.push(quote! { "" });
    header_exprs.push(quote! { "#include <stdint.h>" });
    header_exprs.push(quote! { "#include <stdbool.h>" });
    header_exprs.push(quote! { "#include <string.h>" });
    header_exprs.push(quote! { "" });
    header_exprs.push(quote! { concat!("typedef void* ", #handle_c_name, ";") });
    header_exprs.push(quote! { "" });

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
        if !matches!(self_arg, Some(FnArg::Receiver(_))) {
            continue;
        }

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
        let receiver = match self_arg.unwrap() {
            FnArg::Receiver(r) => r,
            _ => unreachable!(),
        };
        let is_mut = receiver.mutability.is_some();

        let mut param_idents = Vec::new();
        let mut param_name_strs = Vec::new();
        let mut param_kinds = Vec::new();

        for arg in method.sig.inputs.iter().skip(1) {
            let FnArg::Typed(pat_ty) = arg else { continue };
            let Pat::Ident(pat_ident) = &*pat_ty.pat else { continue };
            param_idents.push(pat_ident.ident.clone());
            param_name_strs.push(pat_ident.ident.to_string());

            let kind = if let Some(sk) = classify_ref_type(&pat_ty.ty) {
                uses_slices = true;
                ParamKind::Slice(sk)
            } else {
                ParamKind::Regular(type_tokens_for_macro(
                    &pat_ty.ty,
                    &mut reexport_types,
                    &mut reexport_aliases,
                    &mut alias_counter,
                    &helper_mod_name,
                ))
            };
            param_kinds.push(kind);
        }

        let ret = match &method.sig.output {
            ReturnType::Default => ReturnKind::Void,
            ReturnType::Type(_, ty) => {
                if let Some((ok, err)) = extract_result_types(ty) {
                    let err_ident = type_ident_name(&err);
                    let err_tokens = type_tokens_for_macro(
                        &err,
                        &mut reexport_types,
                        &mut reexport_aliases,
                        &mut alias_counter,
                        &helper_mod_name,
                    );
                    let ok_kind = if is_unit_type(&ok) {
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
            is_mut,
            param_idents,
            param_name_strs,
            param_kinds,
            ret,
            doc_lines,
        });
    }

    // Bytes/Str/Path struct + typedefs (only if used)
    if uses_slices {
        header_exprs.push(quote! { "typedef struct {" });
        header_exprs.push(quote! { "    const char* data;" });
        header_exprs.push(quote! { "    uintptr_t len;" });
        header_exprs.push(quote! { concat!("} ", #bytes_c_name, ";") });
        header_exprs.push(quote! { "" });
        header_exprs.push(quote! {
            concat!("/* Caller must ensure data is valid UTF-8 */")
        });
        header_exprs.push(quote! {
            concat!("typedef ", #bytes_c_name, " ", #str_c_name, ";")
        });
        header_exprs.push(quote! {
            concat!("/* Caller must ensure data is a valid UTF-8 path */")
        });
        header_exprs.push(quote! {
            concat!("typedef ", #bytes_c_name, " ", #path_c_name, ";")
        });
        header_exprs.push(quote! { "" });
        header_exprs.push(quote! {
            concat!("#define ", #str_macro_name, "(s) ((", #str_c_name, "){ .data = (s), .len = strlen(s) })")
        });
        header_exprs.push(quote! { "" });
    }

    // Per-error-type artifacts
    let mut generated_error_types: Vec<String> = Vec::new();

    for m in &methods {
        if let ReturnKind::Result {
            err_ty, err_ident, ..
        } = &m.ret
        {
            if generated_error_types.contains(err_ident) {
                continue;
            }
            generated_error_types.push(err_ident.clone());

            let err_c_name = format!("{type_pfx}{err_ident}");
            let err_snake = camel_to_snake(err_ident);
            let err_upper = camel_to_upper_snake(err_ident);
            let full_upper = format!("{upper_pfx}{err_upper}");

            let message_fn = format_ident!("{fn_pfx}{err_snake}_message");
            let message_fn_str = message_fn.to_string();
            let free_fn = format_ident!("{fn_pfx}{err_snake}_free");
            let free_fn_str = free_fn.to_string();

            header_exprs.push(quote! { "typedef struct {" });
            header_exprs.push(quote! { "    uint64_t code;" });
            header_exprs.push(quote! { "    char* _msg; /* private */" });
            header_exprs.push(quote! { concat!("} ", #err_c_name, ";") });
            header_exprs.push(quote! { "" });

            header_exprs.push(quote! {{
                let mut s = String::new();
                for (name, val) in <#err_ty as ffier::FfiError>::codes() {
                    s.push_str(&format!("#define {}_{} {}\n", #full_upper, name, val));
                }
                s.push_str(&format!("\nconst char* {}(const {}* err);", #message_fn_str, #err_c_name));
                s.push_str(&format!("\nvoid {}({}* err);", #free_fn_str, #err_c_name));
                s
            }});
            header_exprs.push(quote! { "" });

            ffi_fns.push(quote! {
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn #message_fn(
                    err: *const ffier::FfierError,
                ) -> *const core::ffi::c_char {
                    let err = unsafe { &*err };
                    let ptr = err.msg_ptr();
                    if !ptr.is_null() { return ptr; }
                    <#err_ty as ffier::FfiError>::static_message(err.code).as_ptr()
                }
            });

            ffi_fns.push(quote! {
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn #free_fn(err: *mut ffier::FfierError) {
                    unsafe { (*err).free() };
                }
            });
        }
    }

    // Method FFI functions
    for m in &methods {
        let ffi_name_str = &m.ffi_name_str;
        let ffi_name = &m.ffi_name;
        let method_name = &m.method_name;
        let handle_type = format!("{handle_c_name} handle");

        let cast = if m.is_mut {
            quote! { &mut *(handle as *mut $struct_ty) }
        } else {
            quote! { &*(handle as *const $struct_ty) }
        };

        let ffi_params: Vec<_> = m
            .param_idents
            .iter()
            .zip(m.param_kinds.iter())
            .map(|(id, k)| ffi_param_tokens(id, k))
            .collect();

        let converted_args: Vec<_> = m
            .param_idents
            .iter()
            .zip(m.param_kinds.iter())
            .map(|(id, k)| param_conversion(id, k))
            .collect();

        let c_type_exprs: Vec<_> = m
            .param_kinds
            .iter()
            .map(|k| param_c_type_expr(k, &str_c_name, &bytes_c_name, &path_c_name))
            .collect();
        let param_name_str_refs: Vec<_> = m.param_name_strs.iter().collect::<Vec<_>>();

        let method_call = if let Some(tp) = &trait_path {
            quote! { <$struct_ty as $crate::#tp>::#method_name }
        } else {
            quote! { <$struct_ty>::#method_name }
        };

        // Doxygen comment
        let (has_out_param, err_c_name_for_doc) = match &m.ret {
            ReturnKind::Result { ok_ty, err_ident, .. } => {
                (ok_ty.is_some(), Some(format!("{type_pfx}{err_ident}")))
            }
            _ => (false, None),
        };
        if let Some(doc) = build_doxygen_comment(
            &m.doc_lines,
            &m.param_name_strs,
            has_out_param,
            err_c_name_for_doc.as_deref(),
        ) {
            header_exprs.push(quote! { #doc });
        }

        match &m.ret {
            ReturnKind::Void => {
                let header_line = build_header_line(
                    quote! { "void" },
                    ffi_name_str,
                    &handle_type,
                    &c_type_exprs,
                    &param_name_str_refs,
                    None,
                );
                header_exprs.push(header_line);

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        handle: *mut core::ffi::c_void,
                        #(#ffi_params),*
                    ) {
                        let obj = unsafe { #cast };
                        #method_call(obj, #(#converted_args),*);
                    }
                });
            }
            ReturnKind::Value(vk) => {
                let ret_c = value_c_type_expr(vk, &str_c_name, &bytes_c_name, &path_c_name);
                let header_line = build_header_line(
                    ret_c,
                    ffi_name_str,
                    &handle_type,
                    &c_type_exprs,
                    &param_name_str_refs,
                    None,
                );
                header_exprs.push(header_line);

                let ret_ann = value_ret_annotation(vk);
                let into_c = value_into_c(vk);

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        handle: *mut core::ffi::c_void,
                        #(#ffi_params),*
                    ) #ret_ann {
                        let obj = unsafe { #cast };
                        let result = #method_call(obj, #(#converted_args),*);
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

                let out_c_type = ok_ty.as_ref().map(|vk| {
                    value_c_type_expr(vk, &str_c_name, &bytes_c_name, &path_c_name)
                });

                let header_line = build_header_line(
                    quote! { #err_c_name },
                    ffi_name_str,
                    &handle_type,
                    &c_type_exprs,
                    &param_name_str_refs,
                    out_c_type.as_ref(),
                );
                header_exprs.push(header_line);

                let ok_branch = match ok_ty {
                    Some(vk) => {
                        let into_c = value_into_c(vk);
                        let out_write = match vk {
                            ValueKind::Slice(_) => quote! {
                                unsafe { out.write(#into_c) };
                            },
                            ValueKind::Regular(_) => quote! {
                                unsafe { out.write(#into_c) };
                            },
                        };
                        quote! {
                            Ok(result) => {
                                #out_write
                                ffier::FfierError::ok()
                            }
                        }
                    }
                    None => quote! {
                        Ok(()) => ffier::FfierError::ok(),
                    },
                };

                let out_ffi_param = ok_ty.as_ref().map(|vk| match vk {
                    ValueKind::Regular(ty) => {
                        quote! { out: *mut <#ty as ffier::FfiType>::CRepr, }
                    }
                    ValueKind::Slice(_) => {
                        quote! { out: *mut ffier::FfierBytes, }
                    }
                });

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        handle: *mut core::ffi::c_void,
                        #(#ffi_params,)*
                        #out_ffi_param
                    ) -> ffier::FfierError {
                        let obj = unsafe { #cast };
                        match #method_call(obj, #(#converted_args),*) {
                            #ok_branch
                            Err(e) => ffier::FfierError::from_err(e),
                        }
                    }
                });
            }
        }
    }

    // create / destroy
    let create_name = format_ident!("{fn_pfx}{struct_lower}_create");
    let destroy_name = format_ident!("{fn_pfx}{struct_lower}_destroy");
    let create_str = create_name.to_string();
    let destroy_str = destroy_name.to_string();

    header_exprs.push(quote! { "" });
    header_exprs.push(quote! { concat!(#handle_c_name, " ", #create_str, "(void);") });
    header_exprs.push(
        quote! { concat!("void ", #destroy_str, "(", #handle_c_name, " handle);") },
    );

    ffi_fns.push(quote! {
        #[unsafe(no_mangle)]
        pub extern "C" fn #create_name() -> *mut core::ffi::c_void {
            let obj = Box::new(<$struct_ty>::default());
            Box::into_raw(obj) as *mut core::ffi::c_void
        }
    });

    ffi_fns.push(quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #destroy_name(handle: *mut core::ffi::c_void) {
            if !handle.is_null() {
                drop(unsafe { Box::from_raw(handle as *mut $struct_ty) });
            }
        }
    });

    // Close the header guard
    header_exprs.push(quote! { "" });
    header_exprs.push(quote! { concat!("#endif /* ", #header_guard, " */") });

    let header_fn_name = format_ident!("{fn_pfx}{struct_lower}__header");
    let num_lines = header_exprs.len();
    let bridge_macro_name = format_ident!("{struct_lower}_ffier");

    let reexport_items: Vec<_> = reexport_types
        .iter()
        .zip(reexport_aliases.iter())
        .map(|(ty, alias)| quote! { pub type #alias = super::#ty; })
        .collect();

    let output = quote! {
        #impl_block

        #(#warnings)*

        #[doc(hidden)]
        pub mod #helper_mod_name {
            #(#reexport_items)*
        }

        #[macro_export]
        macro_rules! #bridge_macro_name {
            ($struct_ty:ty) => {
                #(#ffi_fns)*

                pub fn #header_fn_name() -> String {
                    let lines: [String; #num_lines] = [
                        #(#header_exprs .to_string()),*
                    ];
                    lines.join("\n")
                }
            };
        }
    };

    output.into()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_header_line(
    c_ret_expr: proc_macro2::TokenStream,
    ffi_name_str: &str,
    handle_type: &str,
    param_c_type_exprs: &[proc_macro2::TokenStream],
    param_name_strs: &[&String],
    out_param_c_type: Option<&proc_macro2::TokenStream>,
) -> proc_macro2::TokenStream {
    let out_snippet = out_param_c_type.map(|ct| {
        quote! {
            s.push_str(", ");
            s.push_str(#ct);
            s.push_str("* out");
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
        s.push_str(#handle_type);
        for (cty, name) in c_type_names.iter().zip(param_names.iter()) {
            s.push_str(", ");
            s.push_str(cty);
            s.push(' ');
            s.push_str(name);
        }
        #out_snippet
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
    }

    let unknown_msg = format!(
        "unknown {} error\0",
        camel_to_snake(&name.to_string()).replace('_', " ")
    );
    let unknown_lit = proc_macro2::Literal::byte_string(unknown_msg.as_bytes());

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

        let code = code.ok_or_else(|| {
            syn::Error::new_spanned(attr, "missing `code` in #[ffier(code = N)]")
        })?;

        return Ok(FfierVariantAttrs { code, message });
    }

    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        "missing #[ffier(code = N)] attribute on variant",
    ))
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

    if sections.body.is_empty()
        && sections.param_docs.is_empty()
        && sections.returns_doc.is_none()
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
        out.push_str(" * @param[out] out Receives the result value on success.\n");
    }

    // @return
    if let Some(err_name) = err_c_name {
        if let Some(ref doc) = sections.returns_doc {
            out.push_str(&format!(" * @return {err_name} — {doc}\n"));
        } else {
            out.push_str(&format!(
                " * @return {err_name} with code 0 on success, error code on failure.\n"
            ));
        }
    } else if let Some(ref doc) = sections.returns_doc {
        out.push_str(&format!(" * @return {doc}\n"));
    }

    out.push_str(" */");
    Some(out)
}
