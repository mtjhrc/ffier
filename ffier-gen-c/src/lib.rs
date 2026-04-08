//! Bridge code generation from parsed metadata.
//!
//! `generate_batch_impl` takes batched metadata token streams and produces
//! the corresponding `extern "C"` FFI functions plus a unified `__ffier_header()`
//! function.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use std::collections::HashMap;

use ffier_meta::{
    MetaError, MetaExportable, MetaImplementable, MetaMethod, MetaParamKind, MetaReceiver,
    MetaReturn, MetaTraitImpl, MetaValueKind, MetaVtableRet, camel_to_snake, camel_to_upper_snake,
    peek_meta_field, peek_meta_name, peek_meta_tag,
};

/// Maps trait names to their concrete dispatch variants.
pub type TraitMap = HashMap<String, TraitDispatchInfo>;

pub struct TraitDispatchInfo {
    pub variants: Vec<TraitVariant>,
    /// If the trait is `#[implementable]`, the wrapper type path and vtable struct path.
    pub implementable: Option<ImplementableInfo>,
}

pub struct TraitVariant {
    pub name: String,
    pub bridge_type: TokenStream2,
}

pub struct ImplementableInfo {
    pub trait_path: TokenStream2,
    pub wrapper_path: TokenStream2,
    pub vtable_struct_path: TokenStream2,
    pub methods: Vec<ffier_meta::MetaVtableMethod>,
}

/// Build the trait-to-impls map from parsed implementable and trait_impl metadata.
fn build_trait_map(
    implementables: &[TokenStream2],
    trait_impls: &[TokenStream2],
) -> TraitMap {
    let mut map = TraitMap::new();

    // trait_impl entries: "Fruit for Apple" → Apple is a concrete implementor
    for item in trait_impls {
        if let Ok(meta) = syn::parse2::<MetaTraitImpl>(item.clone()) {
            let trait_name = meta.trait_name.to_string();
            let struct_name = meta.struct_name.to_string();
            let struct_path = meta.struct_path;
            map.entry(trait_name)
                .or_insert_with(|| TraitDispatchInfo {
                    variants: Vec::new(),
                    implementable: None,
                })
                .variants
                .push(TraitVariant {
                    name: struct_name,
                    bridge_type: struct_path,
                });
        }
    }

    // implementable entries: "trait Fruit" → adds VtableFruit wrapper + stores vtable info
    for item in implementables {
        if let Ok(meta) = syn::parse2::<MetaImplementable>(item.clone()) {
            let trait_name = meta.trait_name.to_string();
            let wrapper_name = format!("Vtable{trait_name}");
            let wrapper_path = meta.wrapper_name.clone();
            let vtable_struct_path = meta.vtable_struct_name.clone();
            let methods = meta.vtable_methods;

            let info = map.entry(trait_name).or_insert_with(|| TraitDispatchInfo {
                variants: Vec::new(),
                implementable: None,
            });
            info.variants.push(TraitVariant {
                name: wrapper_name,
                bridge_type: wrapper_path.clone(),
            });
            info.implementable = Some(ImplementableInfo {
                trait_path: meta.trait_path,
                wrapper_path,
                vtable_struct_path,
                methods,
            });
        }
    }

    map
}

fn generate_one(item: TokenStream2, trait_map: &TraitMap) -> TokenStream2 {
    let tag = peek_meta_tag(&item);
    match tag.as_str() {
        "exportable" => {
            let meta: MetaExportable = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_exportable_bridge(meta, trait_map)
        }
        "error" => {
            let meta: MetaError = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_error_bridge(meta)
        }
        "implementable" => {
            let meta: MetaImplementable = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_implementable_bridge(meta)
        }
        "trait_impl" => {
            let meta: MetaTraitImpl = match syn::parse2(item) {
                Ok(m) => m,
                Err(e) => return e.to_compile_error(),
            };
            generate_trait_impl_bridge(meta)
        }
        _ => {
            let msg = format!("unknown metadata tag `@{tag}`");
            quote! { compile_error!(#msg); }
        }
    }
}

/// Generates bridge code from batched metadata items.
///
/// Input: `{ @tag, ... } { @tag, ... } ...` — multiple brace-delimited items.
/// Sorts into errors → exportables → implementables → trait_impls, generates
/// bridge code for each, and emits a unified `__ffier_header()` function.
pub fn generate_batch_impl(input: TokenStream2) -> TokenStream2 {
    // Split input into individual brace groups
    let mut items: Vec<TokenStream2> = Vec::new();
    for tt in input {
        if let proc_macro2::TokenTree::Group(g) = tt {
            if g.delimiter() == proc_macro2::Delimiter::Brace {
                items.push(g.stream());
            }
        }
    }

    // Sort by category for correct ordering
    let mut errors = Vec::new();
    let mut exportables = Vec::new();
    let mut implementables = Vec::new();
    let mut trait_impls = Vec::new();

    for item in &items {
        match peek_meta_tag(item).as_str() {
            "error" => errors.push(item.clone()),
            "exportable" => exportables.push(item.clone()),
            "implementable" => implementables.push(item.clone()),
            "trait_impl" => trait_impls.push(item.clone()),
            tag => {
                let msg = format!("unknown metadata tag `@{tag}` in batch");
                return quote! { compile_error!(#msg); };
            }
        }
    }

    // Pass 1: Build trait-to-impls map from trait_impl and implementable entries.
    // This allows resolving `impl Trait` params automatically.
    let trait_map = build_trait_map(&implementables, &trait_impls);

    // Pass 2: Generate bridge code for each item in sorted order
    let mut all_code = Vec::new();
    let mut header_fn_names = Vec::new();

    for item in errors
        .iter()
        .chain(exportables.iter())
        .chain(implementables.iter())
        .chain(trait_impls.iter())
    {
        // Collect header function name before generating
        let name = peek_meta_name(item);
        let tag = peek_meta_tag(item);
        let prefix = peek_meta_field(item, "prefix");
        let fn_pfx = format!("{prefix}_");

        let header_fn = if tag == "implementable" {
            let trait_snake = camel_to_snake(&name);
            format_ident!("{fn_pfx}vtable_{trait_snake}__header")
        } else if tag == "trait_impl" {
            let trait_snake = camel_to_snake(&name);
            let struct_name = peek_meta_field(item, "struct_name");
            let struct_snake = camel_to_snake(&struct_name);
            format_ident!("{fn_pfx}{trait_snake}_for_{struct_snake}__header")
        } else {
            let type_snake = camel_to_snake(&name);
            format_ident!("{fn_pfx}{type_snake}__header")
        };
        header_fn_names.push(header_fn);

        all_code.push(generate_one(item.clone(), &trait_map));
    }

    // Generate unified header function
    quote! {
        #(#all_code)*

        pub fn __ffier_header(guard: &str) -> ffier_gen_c::HeaderBuilder {
            ffier_gen_c::HeaderBuilder::new(guard)
                #(.add(#header_fn_names()))*
        }
    }
}

// ===========================================================================
// Exportable bridge generation
// ===========================================================================

fn generate_exportable_bridge(meta: MetaExportable, trait_map: &TraitMap) -> TokenStream2 {
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

    // Generate typedefs for impl Trait dispatch types (resolved from trait map)
    let mut generated_dyn_types: Vec<String> = Vec::new();
    for m in &meta.methods {
        for p in &m.params {
            if let MetaParamKind::ImplTrait { trait_name, .. } = &p.kind {
                let c_name = format!("{type_pfx}{trait_name}");
                if generated_dyn_types.contains(&c_name) {
                    continue;
                }
                generated_dyn_types.push(c_name.clone());

                if let Some(info) = trait_map.get(trait_name) {
                    let variant_names: Vec<String> = info.variants
                        .iter()
                        .map(|v| format!("{type_pfx}{}", v.name))
                        .collect();
                    let variants_comment = variant_names.join(" | ");
                    decl_exprs.push(quote! {
                        format!("typedef void* {}; /* {} */", #c_name, #variants_comment)
                    });
                }
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

        let handle_is_indirect = is_builder && is_by_value;
        let handle_type = if handle_is_indirect {
            format!("{handle_c_name}* handle")
        } else {
            format!("{handle_c_name} handle")
        };

        // Single source of truth: the extern "C" fn signature.
        let c_sig = c_signature_for_method(m, &meta.prefix, SignatureContext::Bridge);

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
                    &type_pfx,
                ));
                header_param_names.push(name);
            }
        }
        let param_name_str_refs: Vec<_> = header_param_names.iter().collect();

        // Collect all impl Trait params with their dispatch info
        struct ImplTraitParam {
            name: syn::Ident,
            dispatch: ffier_meta::DispatchMode,
            trait_name: String,
            variants: Vec<(String, TokenStream2)>,
        }
        let impl_trait_params: Vec<_> = m.params.iter().filter_map(|p| {
            if let MetaParamKind::ImplTrait { trait_name, dispatch } = &p.kind {
                trait_map.get(trait_name).map(|info| ImplTraitParam {
                    name: p.name.clone(),
                    dispatch: *dispatch,
                    trait_name: trait_name.clone(),
                    variants: info.variants.iter()
                        .map(|v| (v.name.clone(), v.bridge_type.clone()))
                        .collect(),
                })
            } else {
                None
            }
        }).collect();

        // Check for dispatch limit (auto mode only)
        let concrete_params: Vec<_> = impl_trait_params.iter()
            .filter(|p| p.dispatch != ffier_meta::DispatchMode::Vtable)
            .collect();
        let total_branches: u64 = concrete_params.iter()
            .map(|p| p.variants.len() as u64)
            .product();
        if total_branches > ffier_meta::DEFAULT_MAX_DISPATCH
            && impl_trait_params.iter().any(|p| p.dispatch == ffier_meta::DispatchMode::Auto)
        {
            let method_name_str = m.name.to_string();
            let msg = format!(
                "ffier: method `{method_name_str}` would generate {total_branches} dispatch \
                 branches (limit: {}). Add `#[ffier(dispatch = vtable)]` to the impl Trait \
                 param(s) or `#[ffier(dispatch = concrete)]` to override the limit.",
                ffier_meta::DEFAULT_MAX_DISPATCH,
            );
            return quote! { compile_error!(#msg); };
        }

        // Check vtable dispatch is possible (trait must be #[implementable])
        for p in &impl_trait_params {
            if p.dispatch == ffier_meta::DispatchMode::Vtable {
                if trait_map.get(&p.trait_name)
                    .and_then(|info| info.implementable.as_ref())
                    .is_none()
                {
                    let msg = format!(
                        "ffier: `#[ffier(dispatch = vtable)]` on param `{}` requires trait `{}` \
                         to have `#[ffier::implementable]`",
                        p.name, p.trait_name,
                    );
                    return quote! { compile_error!(#msg); };
                }
            }
        }

        // Pre-bindings for multi-param types
        let mut pre_bindings = Vec::new();
        let converted_args: Vec<_> = m
            .params
            .iter()
            .map(|p| {
                let id = &p.name;
                match &p.kind {
                    MetaParamKind::ImplTrait { .. } => quote! { #id },
                    MetaParamKind::StrSlice => {
                        let len_name = format!("{}_len", p.name);
                        let len_id = &c_sig.params.iter()
                            .find(|cp| cp.name == len_name)
                            .expect("StrSlice must have _len param in c_sig")
                            .name;
                        let binding = meta_param_conversion(id, &p.kind, Some(len_id));
                        let vec_id = format_ident!("__{id}_vec");
                        pre_bindings.push(quote! { let #vec_id = #binding; });
                        quote! { &#vec_id }
                    }
                    other => meta_param_conversion(id, other, None),
                }
            })
            .collect();

        // Build the method call expression
        let base_method_call = if has_receiver {
            quote! { obj.#method_name(#(#converted_args),*) }
        } else {
            quote! { <#struct_path>::#method_name(#(#converted_args),*) }
        };

        // Determine effective dispatch mode for each param.
        // Auto mode: if total branches ≤ limit, all concrete. Otherwise,
        // first auto param stays concrete, rest become vtable.
        let all_concrete = impl_trait_params.iter()
            .all(|p| p.dispatch != ffier_meta::DispatchMode::Vtable)
            && (total_branches <= ffier_meta::DEFAULT_MAX_DISPATCH
                || impl_trait_params.iter().all(|p| p.dispatch == ffier_meta::DispatchMode::Concrete));
        let effective_dispatch: Vec<bool> = if all_concrete {
            // All concrete dispatch
            vec![false; impl_trait_params.len()]
        } else {
            // Hybrid: explicit concrete stays concrete, explicit vtable stays vtable,
            // auto params: first one concrete, rest vtable
            let mut first_auto_seen = false;
            impl_trait_params.iter().map(|p| match p.dispatch {
                ffier_meta::DispatchMode::Concrete => false, // concrete
                ffier_meta::DispatchMode::Vtable => true,    // vtable
                ffier_meta::DispatchMode::Auto => {
                    if !first_auto_seen {
                        first_auto_seen = true;
                        false // first auto → concrete
                    } else {
                        true // rest → vtable
                    }
                }
            }).collect()
        };

        // Dynamic dispatch via FfierBoxDyn: wrap each vtable-mode param into
        // FfierBoxDyn<dyn Trait>. Linear in variants (N branches per param).
        let mut vtable_pre_bindings: Vec<TokenStream2> = Vec::new();
        for (i, p) in impl_trait_params.iter().enumerate() {
            if !effective_dispatch[i] {
                continue;
            }
            let dyn_id = &p.name;
            let info = trait_map.get(&p.trait_name).unwrap();

            // Check that the trait has dispatch support (implementable or dispatch)
            // The trait map has implementable info if #[ffier::implementable] was used.
            // For #[ffier::dispatch]-only traits, we don't need implementable info —
            // we just need FfierBoxDyn<dyn Trait> to implement Trait.
            // Either way, the codegen is the same: unbox and wrap in FfierBoxDyn.

            let trait_ident = if let Some(imp) = &info.implementable {
                imp.trait_path.clone()
            } else {
                let ident = format_ident!("{}", p.trait_name);
                quote! { #ident }
            };

            let mut branches = Vec::new();
            for v in &info.variants {
                let ty = &v.bridge_type;
                branches.push(quote! {
                    if __type_id == <#ty as ffier::FfiHandle>::type_id() {
                        let __val = unsafe {
                            (*Box::from_raw(#dyn_id as *mut ffier::FfierTaggedBox<#ty>)).value
                        };
                        ffier::FfierBoxDyn(Box::new(__val) as Box<dyn #trait_ident>)
                    }
                });
            }

            let variant_names: Vec<_> = info.variants.iter().map(|v| v.name.as_str()).collect();
            let accepted_list = variant_names.join(" | ");

            vtable_pre_bindings.push(quote! {
                let #dyn_id: ffier::FfierBoxDyn<dyn #trait_ident> = {
                    let __type_id = unsafe { ffier::handle_type_id(#dyn_id) };
                    #(#branches else)* {
                        panic!(
                            "{}(): parameter `{}` expected: {}, got unknown (type_id={:?})",
                            #ffi_name_str, stringify!(#dyn_id), #accepted_list, __type_id,
                        );
                    }
                };
            });
        }

        // Concrete nested dispatch for non-vtable impl Trait params.
        let concrete_impl_trait_params: Vec<_> = impl_trait_params.iter()
            .enumerate()
            .filter(|(i, _)| !effective_dispatch[*i])
            .map(|(_, p)| p)
            .collect();
        let method_call = concrete_impl_trait_params.iter().rev().fold(
            base_method_call,
            |inner, p| {
                let dyn_id = &p.name;
                let variants = &p.variants;
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
                                #inner
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
            },
        );

        // Doxygen comment
        let (has_out_param, err_c_name_for_doc) = match &m.ret {
            MetaReturn::Result { ok, err_ident, .. } => {
                (ok.is_some(), Some(format!("{type_pfx}{err_ident}")))
            }
            _ => (false, None),
        };
        let param_name_strs: Vec<String> = m.params.iter().map(|p| p.name.to_string()).collect();
        // TODO: With trait-based type mapping, detecting handle refs for
        // borrow notes requires compile-time trait evaluation. Skipped for now.
        let borrow_notes: Vec<String> = vec![];
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

        // Extern fn signature from c_sig (shared across all return variants)
        let sig_names: Vec<_> = c_sig.params.iter().map(|p| &p.name).collect();
        let sig_types: Vec<_> = c_sig.params.iter().map(|p| &p.c_type).collect();
        let sig_ret = &c_sig.ret;

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
                        #(#vtable_pre_bindings)*
                        #(#pre_bindings)*
                        let result = #method_call;
                        unsafe { *handle_ptr = <#struct_path as ffier::FfiType>::into_c(result) };
                    }
                } else {
                    quote! {
                        #obj_binding
                        #(#vtable_pre_bindings)*
                        #(#pre_bindings)*
                        #method_call;
                    }
                };

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        #(#sig_names: #sig_types),*
                    ) #sig_ret {
                        #body
                    }
                });
            }
            MetaReturn::Value(vk) => {
                let MetaValueKind::Regular { bridge_type } = vk;
                let ret_c_header = quote! {
                    &ffier_gen_c::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
                };
                let header_line = build_header_line(
                    ret_c_header,
                    &ffi_name_str,
                    header_handle,
                    &c_type_exprs,
                    &param_name_str_refs,
                    None,
                );
                decl_exprs.push(header_line);

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        #(#sig_names: #sig_types),*
                    ) #sig_ret {
                        #obj_binding
                        #(#vtable_pre_bindings)*
                        #(#pre_bindings)*
                        let result = #method_call;
                        <#bridge_type as ffier::FfiType>::into_c(result)
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
                    let MetaValueKind::Regular { bridge_type } = vk;
                    quote! {
                        &ffier_gen_c::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
                    }
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
                        let MetaValueKind::Regular { bridge_type } = vk;
                        quote! {
                            Ok(ok_val) => {
                                unsafe { result.write(<#bridge_type as ffier::FfiType>::into_c(ok_val)) };
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

                let handle_ptr_binding = if handle_is_indirect {
                    quote! { let handle_ptr = handle; }
                } else {
                    quote! {}
                };

                ffi_fns.push(quote! {
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn #ffi_name(
                        #(#sig_names: #sig_types),*
                    ) #sig_ret {
                        #handle_ptr_binding
                        #obj_binding
                        #(#vtable_pre_bindings)*
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
            .map(|p| {
                let id_str = p.name.to_string();
                let bt = &p.bridge_type;
                let type_expr = quote! {
                    &ffier_gen_c::format_c_type_name(<#bt as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
                };
                (id_str, type_expr)
            })
            .collect();

        let ret_c_expr = match &m.ret {
            MetaVtableRet::Void => quote! { "void" },
            MetaVtableRet::Value { bridge_type, .. } => {
                quote! {
                    &ffier_gen_c::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
                }
            }
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

/// Erase named lifetimes from a token stream by parsing as a type,
/// replacing all lifetimes with `'static`, and re-quoting.
/// Used for extern fn signatures where lifetimes aren't in scope.
/// Erase lifetimes from type tokens for extern fn signatures.
/// Replaces named lifetimes with `'static` and adds `'static` to
/// bare references (`&str` → `&'static str`).
fn erase_lifetimes_tokens(tokens: &TokenStream2) -> TokenStream2 {
    if let Ok(ty) = syn::parse2::<syn::Type>(tokens.clone()) {
        struct Eraser;
        impl syn::visit_mut::VisitMut for Eraser {
            fn visit_lifetime_mut(&mut self, lt: &mut syn::Lifetime) {
                *lt = syn::Lifetime::new("'static", lt.apostrophe);
            }
            fn visit_type_reference_mut(&mut self, r: &mut syn::TypeReference) {
                // Add 'static to bare references (elided lifetimes)
                if r.lifetime.is_none() {
                    r.lifetime =
                        Some(syn::Lifetime::new("'static", proc_macro2::Span::call_site()));
                }
                syn::visit_mut::visit_type_reference_mut(self, r);
            }
        }
        let mut ty = ty;
        syn::visit_mut::VisitMut::visit_type_mut(&mut Eraser, &mut ty);
        quote! { #ty }
    } else {
        tokens.clone()
    }
}

/// Extract the Ok type from `Result<OkType, ErrType>` tokens.
pub fn extract_result_ok_type(tokens: &TokenStream2) -> TokenStream2 {
    if let Ok(ty) = syn::parse2::<syn::Type>(tokens.clone())
        && let syn::Type::Path(tp) = &ty
        && let Some(last) = tp.path.segments.last()
        && last.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(syn::GenericArgument::Type(ok_ty)) = args.args.first()
    {
        return quote! { #ok_ty };
    }
    tokens.clone()
}

/// Context for type resolution in extern fn signatures.
///
/// Both contexts resolve to the same C types via `<T as FfiType>::CRepr`,
/// but use different token streams for the type `T`:
/// - `Bridge`: uses `bridge_type` ($crate:: paths that resolve in cdylib)
/// - `Client`: uses `rust_type` (plain types for standalone source)
pub enum SignatureContext {
    /// C bridge in cdylib — types via $crate:: paths
    Bridge,
    /// Standalone Rust client source — types via original names
    Client,
}

/// A single parameter in a C extern signature.
pub struct CExternParam {
    pub name: syn::Ident,
    pub c_type: TokenStream2,
}

/// Complete C extern function signature for a method.
///
/// Contains all the information needed to emit an `unsafe extern "C" { fn ... }`
/// declaration. The parameters include handle, regular params, and out-param
/// (for Result returns) in the order they appear.
pub struct CExternSignature {
    /// Fully qualified extern function name (e.g. "mylib_calculator_add").
    pub fn_name: String,
    /// All parameters in declaration order.
    pub params: Vec<CExternParam>,
    /// Return type tokens (empty for void).
    pub ret: TokenStream2,
}

/// Compute the full C extern signature for a method.
///
/// This is the single source of truth for "what does this method look like
/// as an `extern "C"` function". Both the C bridge generator and the Rust
/// client generator should agree on this.
///
pub fn c_signature_for_method(
    method: &MetaMethod,
    prefix: &str,
    ctx: SignatureContext,
) -> CExternSignature {
    let fn_name = format!("{}_{}", prefix, method.ffi_name);
    let mut params = Vec::new();

    // Handle param (receiver)
    let has_receiver = method.receiver != MetaReceiver::None;
    let is_by_value = method.receiver == MetaReceiver::Value;
    let handle_is_indirect = method.is_builder && is_by_value;

    if has_receiver {
        let c_type = if handle_is_indirect {
            quote! { *mut *mut core::ffi::c_void }
        } else {
            quote! { *mut core::ffi::c_void }
        };
        params.push(CExternParam {
            name: format_ident!("handle"),
            c_type,
        });
    }

    // Regular params
    for p in &method.params {
        if matches!(p.kind, MetaParamKind::StrSlice) {
            params.push(CExternParam {
                name: p.name.clone(),
                c_type: c_param_type(&p.kind, p.rust_type.as_ref(), &ctx),
            });
            params.push(CExternParam {
                name: format_ident!("{}_len", p.name),
                c_type: quote! { usize },
            });
        } else {
            params.push(CExternParam {
                name: p.name.clone(),
                c_type: c_param_type(&p.kind, p.rust_type.as_ref(), &ctx),
            });
        }
    }

    // Return type + out-param for Result
    let ret = match &method.ret {
        MetaReturn::Void => quote! {},
        MetaReturn::Value(vk) => {
            let ty = c_return_type(vk, &method.rust_ret, &ctx);
            quote! { -> #ty }
        }
        MetaReturn::Result { ok, .. } => {
            if let Some(vk) = ok {
                let ok_rust_type = extract_result_ok_type(&method.rust_ret);
                params.push(CExternParam {
                    name: format_ident!("result"),
                    c_type: c_out_param_type(vk, &ok_rust_type, &ctx),
                });
            }
            quote! { -> ffier::FfierError }
        }
    };

    CExternSignature {
        fn_name,
        params,
        ret,
    }
}

/// Produce the concrete C type tokens for a parameter kind.
///
/// This is the canonical "what C type does this parameter have?" function.
/// Both the bridge generator and the client generator use it to ensure
/// their extern declarations agree.
///
/// Produce the C type tokens for a parameter.
pub fn c_param_type(
    kind: &MetaParamKind,
    rust_type: Option<&TokenStream2>,
    ctx: &SignatureContext,
) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular { bridge_type } => {
            let ty = match ctx {
                SignatureContext::Client => {
                    let rt = rust_type.expect("Regular param must have rust_type");
                    erase_lifetimes_tokens(rt)
                }
                SignatureContext::Bridge => bridge_type.clone(),
            };
            quote! { <#ty as ffier::FfiType>::CRepr }
        }
        MetaParamKind::ImplTrait { .. } => quote! { *mut core::ffi::c_void },
        MetaParamKind::StrSlice => quote! { *const ffier::FfierBytes },
    }
}

/// Produce the C return type tokens for a value kind.
pub fn c_return_type(
    kind: &MetaValueKind,
    rust_ret: &TokenStream2,
    ctx: &SignatureContext,
) -> TokenStream2 {
    let MetaValueKind::Regular { bridge_type } = kind;
    let ty = match ctx {
        SignatureContext::Client => erase_lifetimes_tokens(rust_ret),
        SignatureContext::Bridge => bridge_type.clone(),
    };
    quote! { <#ty as ffier::FfiType>::CRepr }
}

/// Produce the C type for a Result ok-value out-parameter.
pub fn c_out_param_type(
    kind: &MetaValueKind,
    rust_ret: &TokenStream2,
    ctx: &SignatureContext,
) -> TokenStream2 {
    let inner = c_return_type(kind, rust_ret, ctx);
    quote! { *mut #inner }
}

fn meta_param_conversion(
    id: &syn::Ident,
    kind: &MetaParamKind,
    len_ident: Option<&syn::Ident>,
) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular { bridge_type } => {
            quote! { <#bridge_type as ffier::FfiType>::from_c(#id) }
        }
        MetaParamKind::StrSlice => {
            let len_id = len_ident.expect("StrSlice conversion needs len_ident");
            quote! { {
                let __slice = unsafe { core::slice::from_raw_parts(#id, #len_id) };
                let __strs: Vec<&str> = __slice.iter().map(|b| unsafe {
                    core::str::from_utf8_unchecked(
                        core::slice::from_raw_parts(b.data, b.len))
                }).collect();
                __strs
            } }
        }
        MetaParamKind::ImplTrait { .. } => {
            quote! { compile_error!("ImplTrait should not use param_conversion") }
        }
    }
}

fn meta_param_c_type_expr(
    kind: &MetaParamKind,
    type_pfx: &str,
) -> TokenStream2 {
    match kind {
        MetaParamKind::Regular { bridge_type } => {
            quote! { &ffier_gen_c::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx) }
        }
        MetaParamKind::StrSlice => {
            quote! { compile_error!("StrSlice should not use param_c_type_expr") }
        }
        MetaParamKind::ImplTrait { trait_name, .. } => {
            let full_name = format!("{type_pfx}{trait_name}");
            quote! { #full_name }
        }
    }
}

/// Format a C type name with the library prefix.
///
/// - Names starting with "Ffier" (e.g. "FfierStr") → replace prefix: "ExStr"
/// - Names starting with uppercase (e.g. "Widget") → prepend prefix: "ExWidget"
/// - Everything else (e.g. "int32_t", "bool") → use as-is
pub fn format_c_type_name(c_type_name: &str, type_pfx: &str) -> String {
    if let Some(suffix) = c_type_name.strip_prefix("Ffier") {
        format!("{type_pfx}{suffix}")
    } else if c_type_name.starts_with(|c: char| c.is_uppercase()) {
        format!("{type_pfx}{c_type_name}")
    } else {
        c_type_name.to_string()
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

// ===========================================================================
// Trait impl bridge generation
// ===========================================================================

fn generate_trait_impl_bridge(meta: MetaTraitImpl) -> TokenStream2 {
    let struct_path = &meta.struct_path;
    let trait_path = &meta.trait_path;
    let fn_pfx = meta.fn_pfx();
    let type_pfx = meta.type_pfx();
    let struct_name_str = meta.struct_name.to_string();
    let struct_snake = camel_to_snake(&struct_name_str);
    let trait_name_str = meta.trait_name.to_string();
    let trait_snake = camel_to_snake(&trait_name_str);

    let header_fn_name = format_ident!("{fn_pfx}{trait_snake}_for_{struct_snake}__header");
    let section_name = format!("{trait_name_str} for {struct_name_str}");

    let mut bridge_fns = Vec::new();
    let mut header_lines: Vec<TokenStream2> = Vec::new();

    for m in &meta.methods {
        let method_name = &m.name;
        let ffi_name_str = format!("{fn_pfx}{struct_snake}_{method_name}");
        let ffi_name = format_ident!("{ffi_name_str}");

        // C params for the bridge function
        let mut bridge_params = vec![quote! { handle: *mut core::ffi::c_void }];
        let mut call_args = Vec::new();

        for p in &m.params {
            let param_name = &p.name;
            let bt = &p.bridge_type;
            bridge_params.push(quote! { #param_name: <#bt as ffier::FfiType>::CRepr });
            call_args.push(quote! { <#bt as ffier::FfiType>::from_c(#param_name) });
        }

        // Return type
        let (ret_type, ret_conversion) = match &m.ret {
            MetaVtableRet::Void => (quote! {}, quote! { call_result }),
            MetaVtableRet::Value { bridge_type, .. } => (
                quote! { -> <#bridge_type as ffier::FfiType>::CRepr },
                quote! { <#bridge_type as ffier::FfiType>::into_c(call_result) },
            ),
        };

        bridge_fns.push(quote! {
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn #ffi_name(#(#bridge_params),*) #ret_type {
                let obj = unsafe { &(*(handle as *const ffier::FfierTaggedBox<#struct_path>)).value };
                let call_result = <#struct_path as #trait_path>::#method_name(obj, #(#call_args),*);
                #ret_conversion
            }
        });

        // Header line
        let param_c_types: Vec<_> = m
            .params
            .iter()
            .map(|p| {
                let id_str = p.name.to_string();
                let bt = &p.bridge_type;
                let type_expr = quote! {
                    &ffier_gen_c::format_c_type_name(<#bt as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
                };
                (id_str, type_expr)
            })
            .collect();

        let ret_c_expr = match &m.ret {
            MetaVtableRet::Void => quote! { "void" },
            MetaVtableRet::Value { bridge_type, .. } => quote! {
                &ffier_gen_c::format_c_type_name(<#bridge_type as ffier::FfiType>::C_TYPE_NAME, #type_pfx)
            },
        };

        let handle_c_name = format!("{type_pfx}{struct_name_str}");
        let param_id_strs: Vec<_> = param_c_types.iter().map(|(id, _)| id.clone()).collect();
        let param_type_exprs: Vec<_> = param_c_types.iter().map(|(_, te)| te.clone()).collect();

        header_lines.push(quote! {{
            let mut s = String::new();
            s.push_str(#ret_c_expr);
            s.push(' ');
            s.push_str(#ffi_name_str);
            s.push('(');
            s.push_str(#handle_c_name);
            s.push_str(" handle");
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

    let num_header_lines = header_lines.len();

    quote! {
        #(#bridge_fns)*

        pub fn #header_fn_name() -> ffier_gen_c::HeaderSection {
            let decl_lines: [String; #num_header_lines] = [
                #(#header_lines .to_string()),*
            ];
            ffier_gen_c::HeaderSection {
                struct_name: #section_name.to_string(),
                handle_typedef: String::new(),
                shared_types: String::new(),
                declarations: decl_lines.join("\n"),
            }
        }
    }
}
