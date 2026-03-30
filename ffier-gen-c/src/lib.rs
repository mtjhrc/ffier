//! Bridge code generation from parsed metadata.
//!
//! `generate_bridge` takes a metadata token stream (starting with `@exportable`,
//! `@error`, or `@implementable`) and produces the corresponding `extern "C"` FFI
//! functions plus a `__header()` function returning a `HeaderSection`.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use ffier_meta::{
    FfiRepr, MetaError, MetaExportable, MetaImplementable, MetaParamKind, MetaReceiver, MetaReturn,
    MetaValueKind, MetaVtableParamType, MetaVtableRetType, camel_to_snake, camel_to_upper_snake,
    peek_meta_tag,
};

/// Generates bridge code (extern "C" FFI functions + header function) from metadata.
///
/// The input token stream must start with one of:
/// - `@exportable, ...` --- generates method FFI functions, destroy, and header
/// - `@error, ...` --- generates error message/free helpers and header
/// - `@implementable, ...` --- generates vtable constructor and header
pub fn generate_bridge_impl(input: TokenStream2) -> TokenStream2 {
    // Peek at the tag to decide which parser to use.
    let tag = peek_meta_tag(&input);

    match tag.as_str() {
        "exportable" => {
            let meta: MetaExportable = match syn::parse2(input) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_exportable_bridge(meta)
        }
        "error" => {
            let meta: MetaError = match syn::parse2(input) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_error_bridge(meta)
        }
        "implementable" => {
            let meta: MetaImplementable = match syn::parse2(input) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_implementable_bridge(meta)
        }
        _ => {
            let msg = format!(
                "unknown metadata tag `@{tag}`: expected @exportable, @error, or @implementable"
            );
            quote! { compile_error!(#msg); }
        }
    }
}

// ===========================================================================
// Exportable bridge generation
// ===========================================================================

fn generate_exportable_bridge(meta: MetaExportable) -> TokenStream2 {
    let struct_path = &meta.struct_path;
    let struct_name = &meta.struct_name.to_string();
    let fn_pfx = meta.fn_pfx();
    let type_pfx = meta.type_pfx();
    let upper_pfx = meta.upper_pfx();
    let handle_c_name = meta.handle_c_name();

    let str_c_name = format!("{type_pfx}Str");
    let bytes_c_name = format!("{type_pfx}Bytes");
    let path_c_name = format!("{type_pfx}Path");
    let str_macro_name = format!("{upper_pfx}STR");

    let struct_lower = camel_to_snake(struct_name);

    // Type aliases: emit `use` statements for bridge types
    let _type_alias_uses: Vec<_> = meta
        .type_aliases
        .iter()
        .map(|(alias, path)| {
            quote! { use #path as #alias; }
        })
        .collect();

    let mut ffi_fns = Vec::new();
    let handle_typedef_expr = quote! { concat!("typedef void* ", #handle_c_name, ";") };
    let mut shared_types_exprs: Vec<TokenStream2> = Vec::new();
    let mut decl_exprs: Vec<TokenStream2> = Vec::new();

    // Bytes/Str/Path struct + typedefs
    if meta.uses_slices() {
        let bytes_macro_name = format!("{upper_pfx}BYTES");

        shared_types_exprs.push(quote! {
            concat!("/* Caller must ensure data is valid UTF-8 */")
        });
        shared_types_exprs.push(quote! { "typedef struct {" });
        shared_types_exprs.push(quote! { "    const char* data;" });
        shared_types_exprs.push(quote! { "    uintptr_t len;" });
        shared_types_exprs.push(quote! { concat!("} ", #str_c_name, ";") });
        shared_types_exprs.push(quote! { "" });

        shared_types_exprs.push(quote! {
            concat!("/* Caller must ensure data is a valid UTF-8 path */")
        });
        shared_types_exprs.push(quote! {
            concat!("typedef ", #str_c_name, " ", #path_c_name, ";")
        });
        shared_types_exprs.push(quote! { "" });

        shared_types_exprs.push(quote! { "typedef struct {" });
        shared_types_exprs.push(quote! { "    const uint8_t* data;" });
        shared_types_exprs.push(quote! { "    uintptr_t len;" });
        shared_types_exprs.push(quote! { concat!("} ", #bytes_c_name, ";") });
        shared_types_exprs.push(quote! { "" });
        shared_types_exprs.push(quote! {
            concat!("#define ", #str_macro_name, "(s) ((", #str_c_name, "){ .data = (s), .len = strlen(s) })")
        });
        // BYTES macro: GNU C (GCC + Clang) gets a statement-expression with a
        // static assert that rejects pointers. Other compilers get a plain
        // version that works correctly but won't catch accidental pointer args.
        shared_types_exprs.push(quote! {
            concat!("#if defined(__GNUC__)")
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
        shared_types_exprs.push(quote! { "#else" });
        shared_types_exprs.push(quote! {
            concat!(
                "#define ", #bytes_macro_name, "(arr) \\")
        });
        shared_types_exprs.push(quote! {
            concat!(
                "    ((", #bytes_c_name, "){ .data = (const uint8_t*)(arr), .len = sizeof(arr) })")
        });
        shared_types_exprs.push(quote! { "#endif" });
    }

    // Generate typedefs for dyn_param dispatch types
    let mut generated_dyn_types: Vec<String> = Vec::new();
    for m in &meta.methods {
        for p in &m.params {
            if let MetaParamKind::DynDispatch {
                c_name_suffix,
                variants,
            } = &p.kind
            {
                let c_name = format!("{type_pfx}{c_name_suffix}");
                if generated_dyn_types.contains(&c_name) {
                    continue;
                }
                generated_dyn_types.push(c_name.clone());

                let variant_names: Vec<String> = variants
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
    for m in &meta.methods {
        let ffi_name_str = format!("{}{}", fn_pfx, m.ffi_name);
        let ffi_name = format_ident!("{}", ffi_name_str);
        let method_name = &m.name;

        let has_receiver = m.receiver != MetaReceiver::None;
        let is_mut = m.receiver == MetaReceiver::Mut;
        let is_by_value = m.receiver == MetaReceiver::Value;
        let is_builder = m.is_builder;

        // Handle parameter
        let handle_is_indirect = is_builder && is_by_value;
        let handle_type = if handle_is_indirect {
            format!("{handle_c_name}* handle")
        } else {
            format!("{handle_c_name} handle")
        };
        let handle_ffi_param = if has_receiver {
            if handle_is_indirect {
                Some(quote! { handle: *mut *mut core::ffi::c_void, })
            } else {
                Some(quote! { handle: *mut core::ffi::c_void, })
            }
        } else {
            None
        };

        // Self cast via FfierTaggedBox (instance methods only)
        let obj_binding = if has_receiver {
            let handle_deref = if handle_is_indirect {
                quote! { let handle = unsafe { *handle }; }
            } else {
                quote! {}
            };
            let type_assert = quote! {
                #handle_deref
                let __actual = unsafe { ffier::handle_type_id(handle) };
                let __expected = <#struct_path as ffier::FfiHandle>::type_id();
                assert!(
                    __actual == __expected,
                    "{}(): `handle` is not a {} (expected type_id={:?}, got {:?})",
                    #ffi_name_str,
                    <#struct_path as ffier::FfiHandle>::C_HANDLE_NAME,
                    __expected,
                    __actual,
                );
            };
            let cast = if is_by_value {
                quote! {
                    let tagged = *Box::from_raw(
                        handle as *mut ffier::FfierTaggedBox<#struct_path>
                    );
                    tagged.value
                }
            } else if is_mut {
                quote! {
                    &mut (*(handle as *mut ffier::FfierTaggedBox<#struct_path>)).value
                }
            } else {
                quote! {
                    &(*(handle as *const ffier::FfierTaggedBox<#struct_path>)).value
                }
            };
            Some(quote! { #type_assert let obj = unsafe { #cast }; })
        } else {
            None
        };

        let ffi_params: Vec<_> = m
            .params
            .iter()
            .map(|p| meta_ffi_param_tokens(&p.name, &p.kind))
            .collect();

        let mut c_type_exprs = Vec::new();
        let mut header_param_names: Vec<String> = Vec::new();
        for p in &m.params {
            let name = p.name.to_string();
            if matches!(p.kind, MetaParamKind::StrSlice) {
                let ptr_type = format!("const {str_c_name}*");
                c_type_exprs.push(quote! { #ptr_type });
                header_param_names.push(name.clone());
                c_type_exprs.push(quote! { "uintptr_t" });
                header_param_names.push(format!("{name}_len"));
            } else {
                c_type_exprs.push(meta_param_c_type_expr(
                    &p.kind,
                    &str_c_name,
                    &bytes_c_name,
                    &path_c_name,
                    &type_pfx,
                ));
                header_param_names.push(name);
            }
        }
        let param_name_str_refs: Vec<_> = header_param_names.iter().collect();

        // Check for DynDispatch params
        let dyn_dispatch = m.params.iter().find_map(|p| match &p.kind {
            MetaParamKind::DynDispatch {
                c_name_suffix,
                variants,
            } => Some((
                p.name.clone(),
                format!("{type_pfx}{c_name_suffix}"),
                variants.clone(),
            )),
            _ => None,
        });

        // Pre-bindings for multi-param types
        let mut pre_bindings = Vec::new();
        let converted_args: Vec<_> = m
            .params
            .iter()
            .map(|p| {
                let id = &p.name;
                match &p.kind {
                    MetaParamKind::DynDispatch { .. } => quote! { #id },
                    MetaParamKind::StrSlice => {
                        let binding = meta_param_conversion(id, &p.kind);
                        let vec_id = format_ident!("__{id}_vec");
                        pre_bindings.push(quote! { let #vec_id = #binding; });
                        quote! { &#vec_id }
                    }
                    other => meta_param_conversion(id, other),
                }
            })
            .collect();

        // Build the method call expression
        let base_method_call = if has_receiver {
            quote! { obj.#method_name(#(#converted_args),*) }
        } else {
            quote! { <#struct_path>::#method_name(#(#converted_args),*) }
        };

        // Wrap in dispatch match if needed
        let method_call = if let Some((ref dyn_id, ref _c_name, ref variants)) = dyn_dispatch {
            let if_branches: Vec<_> = variants
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

            let variant_names: Vec<_> = variants.iter().map(|(name, _)| name.as_str()).collect();
            let accepted_list = variant_names.join(" | ");

            quote! {{
                let __type_id = unsafe { ffier::handle_type_id(#dyn_id) };
                #(#if_branches else)* {
                    panic!(
                        "{}(): parameter `{}` expected an object of type: {}, \
                         but got unknown handle (type_id={:?})",
                        #ffi_name_str,
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
            MetaReturn::Result { ok, err_ident, .. } => {
                (ok.is_some(), Some(format!("{type_pfx}{err_ident}")))
            }
            _ => (false, None),
        };
        let param_name_strs: Vec<String> = m.params.iter().map(|p| p.name.to_string()).collect();
        let borrow_notes: Vec<String> =
            if !meta.lifetimes.is_empty() && m.receiver == MetaReceiver::None {
                m.params
                    .iter()
                    .filter_map(|p| {
                        if matches!(p.kind, MetaParamKind::HandleRef { .. }) {
                            Some(format!(
                                "`{}` is borrowed by the returned `{}`. \
                                 It must not be directly modified or destroyed while the `{}` is alive.",
                                p.name, handle_c_name, handle_c_name
                            ))
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                vec![]
            };
        if let Some(doc) = build_doxygen_comment(
            &m.doc,
            &param_name_strs,
            has_out_param,
            err_c_name_for_doc.as_deref(),
            &borrow_notes,
        ) {
            decl_exprs.push(quote! { #doc });
        }

        let header_handle = if has_receiver {
            Some(&handle_type)
        } else {
            None
        };

        match &m.ret {
            MetaReturn::Void => {
                let header_line = build_header_line(
                    quote! { "void" },
                    &ffi_name_str,
                    header_handle,
                    &c_type_exprs,
                    &param_name_str_refs,
                    None,
                );
                decl_exprs.push(header_line);

                let body = if handle_is_indirect {
                    quote! {
                        let handle_ptr = handle;
                        #obj_binding
                        #(#pre_bindings)*
                        let result = #method_call;
                        unsafe { *handle_ptr = <#struct_path as ffier::FfiType>::into_c(result) };
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
            MetaReturn::Value(vk) => {
                let ret_c =
                    meta_value_c_type_expr(vk, &str_c_name, &bytes_c_name, &path_c_name, &type_pfx);
                let header_line = build_header_line(
                    ret_c,
                    &ffi_name_str,
                    header_handle,
                    &c_type_exprs,
                    &param_name_str_refs,
                    None,
                );
                decl_exprs.push(header_line);

                let ret_ann = meta_value_ret_annotation(vk);
                let result_ident = format_ident!("result");
                let into_c = meta_value_into_c(vk, &result_ident);

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
            MetaReturn::Result {
                ok,
                err_bridge_type: _,
                err_ident,
            } => {
                let err_c_name = format!("{type_pfx}{err_ident}");

                let out_c_type = ok.as_ref().map(|vk| {
                    meta_value_c_type_expr(vk, &str_c_name, &bytes_c_name, &path_c_name, &type_pfx)
                });

                let header_line = build_header_line(
                    quote! { #err_c_name },
                    &ffi_name_str,
                    header_handle,
                    &c_type_exprs,
                    &param_name_str_refs,
                    out_c_type.as_ref(),
                );
                decl_exprs.push(header_line);

                let ok_branch = match ok {
                    Some(vk) => {
                        let ok_val_ident = format_ident!("ok_val");
                        let into_c = meta_value_into_c(vk, &ok_val_ident);
                        quote! {
                            Ok(ok_val) => {
                                unsafe { result.write(#into_c) };
                                ffier::FfierError::ok()
                            }
                        }
                    }
                    None if handle_is_indirect => quote! {
                        Ok(new_self) => {
                            unsafe { *handle_ptr = <#struct_path as ffier::FfiType>::into_c(new_self) };
                            ffier::FfierError::ok()
                        }
                    },
                    None => quote! {
                        Ok(_) => ffier::FfierError::ok(),
                    },
                };

                let out_ffi_param = ok.as_ref().map(|vk| match vk {
                    MetaValueKind::Regular { bridge_type, .. } => {
                        quote! { result: *mut <#bridge_type as ffier::FfiType>::CRepr, }
                    }
                    MetaValueKind::SliceStr
                    | MetaValueKind::SliceBytes
                    | MetaValueKind::SlicePath => {
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

    // destroy function
    let destroy_name = format_ident!("{fn_pfx}{struct_lower}_destroy");
    let destroy_str = destroy_name.to_string();

    decl_exprs.push(quote! { concat!("void ", #destroy_str, "(", #handle_c_name, " handle);") });

    ffi_fns.push(quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #destroy_name(handle: *mut core::ffi::c_void) {
            if !handle.is_null() {
                let __actual = unsafe { ffier::handle_type_id(handle) };
                let __expected = <#struct_path as ffier::FfiHandle>::type_id();
                assert!(
                    __actual == __expected,
                    "{}(): `handle` is not a {} (expected type_id={:?}, got {:?})",
                    #destroy_str,
                    <#struct_path as ffier::FfiHandle>::C_HANDLE_NAME,
                    __expected,
                    __actual,
                );
                drop(unsafe {
                    Box::from_raw(handle as *mut ffier::FfierTaggedBox<#struct_path>)
                });
            }
        }
    });

    // Header function
    let header_fn_name = format_ident!("{fn_pfx}{struct_lower}__header");
    let num_shared = shared_types_exprs.len();
    let num_decls = decl_exprs.len();

    quote! {
        #(#ffi_fns)*

        pub fn #header_fn_name() -> ffier_gen_c::HeaderSection {
            let handle_typedef = #handle_typedef_expr .to_string();
            let shared_lines: [String; #num_shared] = [
                #(#shared_types_exprs .to_string()),*
            ];
            let shared_types = shared_lines.join("\n");
            let decl_lines: [String; #num_decls] = [
                #(#decl_exprs .to_string()),*
            ];
            let declarations = decl_lines.join("\n");
            ffier_gen_c::HeaderSection {
                struct_name: #struct_name.to_string(),
                handle_typedef,
                shared_types,
                declarations,
            }
        }
    }
}

// ===========================================================================
// Error bridge generation
// ===========================================================================

fn generate_error_bridge(meta: MetaError) -> TokenStream2 {
    let name = &meta.name;
    let path = &meta.path;
    let fn_pfx = meta.fn_pfx();
    let type_pfx = meta.type_pfx();
    let upper_pfx = meta.upper_pfx();

    let name_str = name.to_string();
    let err_snake = camel_to_snake(&name_str);
    let err_upper = camel_to_upper_snake(&name_str);

    let message_fn_name = format_ident!("{fn_pfx}{err_snake}_message");
    let free_fn_name = format_ident!("{fn_pfx}{err_snake}_free");
    let header_fn_name = format_ident!("{fn_pfx}{err_snake}__header");

    let err_c_name = format!("{type_pfx}{name_str}");
    let message_fn_str = format!("{fn_pfx}{err_snake}_message");
    let free_fn_str = format!("{fn_pfx}{err_snake}_free");
    let full_upper_pfx = format!("{upper_pfx}{err_upper}");

    quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #message_fn_name(
            err: *const ffier::FfierError,
        ) -> *const core::ffi::c_char {
            let err = unsafe { &*err };
            let ptr = err.msg_ptr();
            if !ptr.is_null() { return ptr; }
            <#path as ffier::FfiError>::static_message(err.code).as_ptr()
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #free_fn_name(
            err: *mut ffier::FfierError,
        ) {
            unsafe { (*err).free() };
        }

        pub fn #header_fn_name() -> ffier_gen_c::HeaderSection {
            let err_c_name = #err_c_name;
            let message_fn_str = #message_fn_str;
            let free_fn_str = #free_fn_str;
            let full_upper_pfx = #full_upper_pfx;

            let mut decls = String::new();
            decls.push_str("typedef struct {\n");
            decls.push_str("    uint64_t code;\n");
            decls.push_str("    char* _msg; /* private */\n");
            decls.push_str(&format!("}} {};\n\n", err_c_name));

            for (variant_name, val) in <#path as ffier::FfiError>::codes() {
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

            ffier_gen_c::HeaderSection {
                struct_name: #name_str.to_string(),
                handle_typedef: String::new(),
                shared_types: String::new(),
                declarations: decls,
            }
        }
    }
}

// ===========================================================================
// Implementable bridge generation
// ===========================================================================

fn generate_implementable_bridge(meta: MetaImplementable) -> TokenStream2 {
    let vtable_struct_name = &meta.vtable_struct_name;
    let wrapper_name = &meta.wrapper_name;
    let vtable_c_name = meta.vtable_c_name();
    let type_pfx = meta.type_pfx();
    let fn_pfx = meta.fn_pfx();
    let trait_path = &meta.trait_path;
    let _ = trait_path; // available if needed for qualified paths

    let constructor_name_str = meta.constructor_name();
    let constructor_name = format_ident!("{}", constructor_name_str);

    let trait_name_str = meta.trait_name.to_string();
    let trait_snake = camel_to_snake(&trait_name_str);
    let header_fn_name = format_ident!("{fn_pfx}vtable_{trait_snake}__header");
    let vtable_section_name = format!("Vtable{trait_name_str}");

    // Build header lines for vtable struct
    let mut header_lines: Vec<TokenStream2> = Vec::new();

    header_lines.push(quote! { concat!("typedef struct {") });

    // For each method, generate a C function pointer line
    for m in &meta.vtable_methods {
        let name_str = m.name.to_string();
        let param_c_types: Vec<_> = m
            .params
            .iter()
            .map(|(id, vpt)| {
                let id_str = id.to_string();
                let type_expr = match vpt {
                    MetaVtableParamType::Primitive(ty) => {
                        quote! { <#ty as ffier::FfiType>::C_TYPE_NAME }
                    }
                    MetaVtableParamType::Str => {
                        let n = format!("{type_pfx}Str");
                        quote! { #n }
                    }
                    MetaVtableParamType::Bytes => {
                        let n = format!("{type_pfx}Bytes");
                        quote! { #n }
                    }
                    MetaVtableParamType::Path => {
                        let n = format!("{type_pfx}Path");
                        quote! { #n }
                    }
                    MetaVtableParamType::Handle(_) => quote! { "void*" },
                };
                (id_str, type_expr)
            })
            .collect();

        let ret_c_expr = match &m.ret {
            MetaVtableRetType::Void => quote! { "void" },
            MetaVtableRetType::Primitive(ty) => {
                quote! { <#ty as ffier::FfiType>::C_TYPE_NAME }
            }
            MetaVtableRetType::Str => {
                let n = format!("{type_pfx}Str");
                quote! { #n }
            }
            MetaVtableRetType::Bytes => {
                let n = format!("{type_pfx}Bytes");
                quote! { #n }
            }
            MetaVtableRetType::Path => {
                let n = format!("{type_pfx}Path");
                quote! { #n }
            }
            MetaVtableRetType::Handle(_) => quote! { "void*" },
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

    quote! {
        #[unsafe(no_mangle)]
        pub extern "C" fn #constructor_name(
            user_data: *mut core::ffi::c_void,
            vtable: *const #vtable_struct_name,
        ) -> *mut core::ffi::c_void {
            let wrapper = #wrapper_name {
                user_data,
                vtable,
            };
            <#wrapper_name as ffier::FfiType>::into_c(wrapper)
        }

        pub fn #header_fn_name() -> ffier_gen_c::HeaderSection {
            let decl_lines: [String; #num_header_lines] = [
                #(#header_lines .to_string()),*
            ];
            let declarations = decl_lines.join("\n");
            ffier_gen_c::HeaderSection {
                struct_name: #vtable_section_name.to_string(),
                handle_typedef: String::new(),
                shared_types: String::new(),
                declarations,
            }
        }
    }
}

// ===========================================================================
// Shared C ABI type resolution — used by both ffier-gen-c and ffier-gen-rust
// ===========================================================================

/// Produce the concrete C type tokens for a parameter kind.
///
/// This is the canonical "what C type does this parameter have?" function.
/// Both the bridge generator and the client generator use it to ensure
/// their extern declarations agree.
pub fn c_param_type(kind: &MetaParamKind) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular { bridge_type, repr } => match repr {
            FfiRepr::Primitive => quote! { #bridge_type },
            FfiRepr::Handle => quote! { *mut core::ffi::c_void },
            FfiRepr::Other(c_repr) => quote! { #c_repr },
        },
        MetaParamKind::SliceStr | MetaParamKind::SliceBytes | MetaParamKind::SlicePath => {
            quote! { ffier::FfierBytes }
        }
        MetaParamKind::HandleRef { .. } | MetaParamKind::DynDispatch { .. } => {
            quote! { *mut core::ffi::c_void }
        }
        MetaParamKind::StrSlice => {
            // StrSlice expands to two params — this returns the type of the first one.
            // Callers must handle the second (len: usize) separately.
            quote! { *const ffier::FfierBytes }
        }
    }
}

/// Produce the concrete C return type tokens for a value kind.
pub fn c_return_type(kind: &MetaValueKind) -> TokenStream2 {
    match kind {
        MetaValueKind::Regular { bridge_type, repr } => match repr {
            FfiRepr::Primitive => quote! { #bridge_type },
            FfiRepr::Handle => quote! { *mut core::ffi::c_void },
            FfiRepr::Other(c_repr) => quote! { #c_repr },
        },
        MetaValueKind::SliceStr | MetaValueKind::SliceBytes | MetaValueKind::SlicePath => {
            quote! { ffier::FfierBytes }
        }
    }
}

/// Produce the concrete C type for a Result ok-value out-parameter.
pub fn c_out_param_type(kind: &MetaValueKind) -> TokenStream2 {
    let inner = c_return_type(kind);
    quote! { *mut #inner }
}

// ===========================================================================
// Bridge-specific helpers
// ===========================================================================

fn meta_ffi_param_tokens(id: &syn::Ident, kind: &MetaParamKind) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular {
            bridge_type,
            repr: _,
        } => quote! { #id: <#bridge_type as ffier::FfiType>::CRepr },
        MetaParamKind::SliceStr | MetaParamKind::SliceBytes | MetaParamKind::SlicePath => {
            quote! { #id: ffier::FfierBytes }
        }
        MetaParamKind::StrSlice => {
            let len_id = format_ident!("{id}_len");
            quote! { #id: *const ffier::FfierBytes, #len_id: usize }
        }
        MetaParamKind::HandleRef { .. } | MetaParamKind::DynDispatch { .. } => {
            quote! { #id: *mut core::ffi::c_void }
        }
    }
}

fn meta_param_conversion(id: &syn::Ident, kind: &MetaParamKind) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular {
            bridge_type,
            repr: _,
        } => quote! { <#bridge_type as ffier::FfiType>::from_c(#id) },
        MetaParamKind::SliceStr => quote! { unsafe {
            core::str::from_utf8_unchecked(
                core::slice::from_raw_parts(#id.data, #id.len))
        } },
        MetaParamKind::SliceBytes => quote! { unsafe {
            core::slice::from_raw_parts(#id.data, #id.len)
        } },
        MetaParamKind::SlicePath => quote! { unsafe { #id.as_path() } },
        MetaParamKind::StrSlice => {
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
        MetaParamKind::HandleRef {
            bridge_type,
            is_mut: true,
        } => {
            quote! { unsafe {
                &mut (*(#id as *mut ffier::FfierTaggedBox<#bridge_type>)).value
            } }
        }
        MetaParamKind::HandleRef {
            bridge_type,
            is_mut: false,
        } => {
            quote! { unsafe {
                &(*(#id as *const ffier::FfierTaggedBox<#bridge_type>)).value
            } }
        }
        MetaParamKind::DynDispatch { .. } => {
            quote! { compile_error!("DynDispatch should not use param_conversion") }
        }
    }
}

fn meta_param_c_type_expr(
    kind: &MetaParamKind,
    str_name: &str,
    bytes_name: &str,
    path_name: &str,
    type_pfx: &str,
) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular {
            bridge_type, repr, ..
        } => {
            match repr {
                FfiRepr::Handle => {
                    // Handle types: prepend type_pfx to the struct name at runtime
                    quote! { &format!("{}{}", #type_pfx, <#bridge_type as ffier::FfiType>::C_TYPE_NAME) }
                }
                _ => {
                    // Primitives & Other: C_TYPE_NAME already correct
                    quote! { <#bridge_type as ffier::FfiType>::C_TYPE_NAME }
                }
            }
        }
        MetaParamKind::SliceStr => quote! { #str_name },
        MetaParamKind::SliceBytes => quote! { #bytes_name },
        MetaParamKind::SlicePath => quote! { #path_name },
        MetaParamKind::StrSlice => {
            quote! { compile_error!("StrSlice should not use param_c_type_expr") }
        }
        MetaParamKind::HandleRef { bridge_type, .. } => {
            // Handle ref: prepend type_pfx at runtime
            quote! { &format!("{}{}", #type_pfx, <#bridge_type as ffier::FfiHandle>::C_HANDLE_NAME) }
        }
        MetaParamKind::DynDispatch { c_name_suffix, .. } => {
            let full_name = format!("{type_pfx}{c_name_suffix}");
            quote! { #full_name }
        }
    }
}

fn meta_value_ret_annotation(kind: &MetaValueKind) -> TokenStream2 {
    match kind {
        MetaValueKind::Regular { bridge_type, .. } => {
            quote! { -> <#bridge_type as ffier::FfiType>::CRepr }
        }
        MetaValueKind::SliceStr | MetaValueKind::SliceBytes | MetaValueKind::SlicePath => {
            quote! { -> ffier::FfierBytes }
        }
    }
}

fn meta_value_into_c(kind: &MetaValueKind, var: &syn::Ident) -> TokenStream2 {
    match kind {
        MetaValueKind::Regular { bridge_type, .. } => {
            quote! { <#bridge_type as ffier::FfiType>::into_c(#var) }
        }
        MetaValueKind::SliceStr => quote! { ffier::FfierBytes::from_str(#var) },
        MetaValueKind::SliceBytes => quote! { ffier::FfierBytes::from_bytes(#var) },
        MetaValueKind::SlicePath => quote! { ffier::FfierBytes::from_path(#var) },
    }
}

fn meta_value_c_type_expr(
    kind: &MetaValueKind,
    str_name: &str,
    bytes_name: &str,
    path_name: &str,
    type_pfx: &str,
) -> TokenStream2 {
    match kind {
        MetaValueKind::Regular {
            bridge_type, repr, ..
        } => match repr {
            FfiRepr::Handle => {
                quote! { &format!("{}{}", #type_pfx, <#bridge_type as ffier::FfiType>::C_TYPE_NAME) }
            }
            _ => {
                quote! { <#bridge_type as ffier::FfiType>::C_TYPE_NAME }
            }
        },
        MetaValueKind::SliceStr => quote! { #str_name },
        MetaValueKind::SliceBytes => quote! { #bytes_name },
        MetaValueKind::SlicePath => quote! { #path_name },
    }
}

// ---------------------------------------------------------------------------
// Header line + doxygen helpers (ported from lib.rs for standalone use)
// ---------------------------------------------------------------------------

fn build_header_line(
    c_ret_expr: TokenStream2,
    ffi_name_str: &str,
    handle_type: Option<&String>,
    param_c_type_exprs: &[TokenStream2],
    param_name_strs: &[&String],
    out_param_c_type: Option<&TokenStream2>,
) -> TokenStream2 {
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

/// Parsed doc comment sections.
struct DocSections {
    body: Vec<String>,
    param_docs: Vec<(String, String)>,
    returns_doc: Option<String>,
}

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

        let lower = line.trim().to_lowercase();
        if lower == "# arguments" || lower == "# parameters" {
            section = Section::Arguments;
            continue;
        }
        if lower.starts_with("# return") {
            section = Section::Returns;
            continue;
        }
        if line.trim().starts_with("# ") {
            if !returns_lines.is_empty() {
                returns_doc = Some(returns_lines.join(" ").trim().to_string());
                returns_lines.clear();
            }
            section = Section::Body;
        }

        match section {
            Section::Body => body.push(raw.clone()),
            Section::Arguments => {
                let trimmed = line.trim();
                let after_bullet = trimmed
                    .strip_prefix("* ")
                    .or_else(|| trimmed.strip_prefix("- "));
                if let Some(rest) = after_bullet
                    && let Some((name, desc)) = parse_param_entry(rest)
                {
                    param_docs.push((name, desc));
                }
            }
            Section::Returns => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    returns_lines.push(trimmed.to_string());
                }
            }
        }
    }

    if !returns_lines.is_empty() {
        returns_doc = Some(returns_lines.join(" ").trim().to_string());
    }

    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }

    DocSections {
        body,
        param_docs,
        returns_doc,
    }
}

fn parse_param_entry(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let rest = s.strip_prefix('`')?;
    let end = rest.find('`')?;
    let name = rest[..end].to_string();
    let after = rest[end + 1..].trim();
    let desc = after.strip_prefix('-').unwrap_or(after).trim().to_string();
    Some((name, desc))
}

fn build_doxygen_comment(
    doc_lines: &[String],
    param_names: &[String],
    has_out_param: bool,
    err_c_name: Option<&str>,
    borrow_notes: &[String],
) -> Option<String> {
    if doc_lines.is_empty() && borrow_notes.is_empty() {
        return None;
    }

    let sections = parse_doc_sections(doc_lines);

    if sections.body.is_empty() && sections.param_docs.is_empty() && sections.returns_doc.is_none()
    {
        return None;
    }

    let mut out = String::from("/**\n");

    for line in &sections.body {
        let trimmed = line.strip_prefix(' ').unwrap_or(line);
        if trimmed.is_empty() {
            out.push_str(" *\n");
        } else {
            out.push_str(&format!(" * {trimmed}\n"));
        }
    }

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

    for note in borrow_notes {
        out.push_str(&format!(" * @note {note}\n"));
    }

    if has_out_param {
        if let Some(ref doc) = sections.returns_doc {
            out.push_str(&format!(" * @param[out] result {doc}\n"));
        } else {
            out.push_str(" * @param[out] result\n");
        }
    }

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
// HeaderSection / HeaderBuilder --- structured C header generation
// ===========================================================================

pub struct HeaderSection {
    pub struct_name: String,
    pub handle_typedef: String,
    pub shared_types: String,
    pub declarations: String,
}

pub struct HeaderBuilder {
    guard: String,
    sections: Vec<HeaderSection>,
}

// TODO: HeaderBuilder should accept the prefix (e.g. "krun") instead of a raw guard string,
// and derive the header guard, handle typedefs, shared type names, and macro names from it.
// The prefix should only be specified here, not duplicated in each #[ffier::exportable(prefix = "krun")].

impl HeaderBuilder {
    pub fn new(guard: &str) -> Self {
        Self {
            guard: guard.to_string(),
            sections: Vec::new(),
        }
    }

    pub fn add(mut self, section: HeaderSection) -> Self {
        self.sections.push(section);
        self
    }

    pub fn build(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!("#ifndef {}\n", self.guard));
        out.push_str(&format!("#define {}\n", self.guard));
        out.push('\n');
        out.push_str("#include <stdint.h>\n");
        out.push_str("#include <stdbool.h>\n");
        out.push_str("#include <string.h>\n");
        out.push('\n');

        // Collect all handle typedefs
        let mut has_handle = false;
        for section in &self.sections {
            if !section.handle_typedef.is_empty() {
                out.push_str(&section.handle_typedef);
                out.push('\n');
                has_handle = true;
            }
        }
        if has_handle {
            out.push('\n');
        }

        // Emit shared types from the first section that has them
        for section in &self.sections {
            if !section.shared_types.is_empty() {
                out.push_str(&section.shared_types);
                out.push('\n');
                break;
            }
        }

        out.push_str("/* Header auto-generated by ffier */\n");

        // Per-section declarations
        for section in &self.sections {
            if !section.declarations.is_empty() {
                // Ensure blank line before section comment
                if !out.ends_with("\n\n") {
                    if out.ends_with('\n') {
                        out.push('\n');
                    } else {
                        out.push_str("\n\n");
                    }
                }
                let name = &section.struct_name;
                let dashes = "-".repeat(72 - 6 - name.len()); // 72 cols total
                out.push_str(&format!("/* {name} {dashes} */\n\n"));
                out.push_str(&section.declarations);
            }
        }

        out.push('\n');
        out.push_str(&format!("#endif /* {} */\n", self.guard));
        out
    }
}
