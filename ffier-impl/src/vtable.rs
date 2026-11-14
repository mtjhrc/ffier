//! Vtable struct and wrapper trait implementation generation.

use std::collections::{BTreeMap, HashMap};

use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Token, parse::Parse, parse_macro_input};

/// Input format:
/// ```ignore
/// vtable_struct = Ident;
/// wrapper = Ident;
/// trait_path = (token tokens);
/// trait_generics = (token tokens);  // e.g. <'static> or empty
/// crate_path = (token tokens);      // typically $crate
/// handles = [Ident, Ident, ...];
/// num_slots = N;
/// methods = [ {method_meta}, ... ];
/// default_helpers = [ method_name => (path tokens), ... ];
/// ```
struct GenerateVtableInput {
    vtable_struct: syn::Ident,
    wrapper: syn::Ident,
    trait_path: proc_macro2::TokenStream,
    trait_generics: proc_macro2::TokenStream,
    crate_path: proc_macro2::TokenStream,
    handles: Vec<syn::Ident>,
    values: Vec<syn::Ident>,
    num_slots: usize,
    own_method_count: usize,
    methods: Vec<proc_macro2::TokenStream>,
    method_sigs: Vec<syn::Signature>,
    default_helpers: Vec<(syn::Ident, proc_macro2::TokenStream)>,
}

impl Parse for GenerateVtableInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut vtable_struct = None;
        let mut wrapper = None;
        let mut trait_path = None;
        let mut trait_generics = None;
        let mut crate_path = None;
        let mut handles = Vec::new();
        let mut values = Vec::new();
        let mut own_method_count = 0usize;
        let mut num_slots = None;
        let mut methods: Vec<proc_macro2::TokenStream> = Vec::new();
        let mut method_sigs: Vec<syn::Signature> = Vec::new();
        let mut default_helpers = Vec::new();

        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "vtable_struct" => {
                    vtable_struct = Some(input.parse::<syn::Ident>()?);
                    input.parse::<Token![;]>()?;
                }
                "wrapper" => {
                    wrapper = Some(input.parse::<syn::Ident>()?);
                    input.parse::<Token![;]>()?;
                }
                "trait_path" => {
                    let content;
                    syn::parenthesized!(content in input);
                    trait_path = Some(content.parse::<proc_macro2::TokenStream>()?);
                    input.parse::<Token![;]>()?;
                }
                "trait_generics" => {
                    let content;
                    syn::parenthesized!(content in input);
                    trait_generics = Some(content.parse::<proc_macro2::TokenStream>()?);
                    input.parse::<Token![;]>()?;
                }
                "crate_path" => {
                    let content;
                    syn::parenthesized!(content in input);
                    crate_path = Some(content.parse::<proc_macro2::TokenStream>()?);
                    input.parse::<Token![;]>()?;
                }
                "handles" => {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        handles.push(content.parse::<syn::Ident>()?);
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    input.parse::<Token![;]>()?;
                }

                "values" => {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        values.push(content.parse::<syn::Ident>()?);
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    input.parse::<Token![;]>()?;
                }

                "own_method_count" => {
                    own_method_count = input.parse::<syn::LitInt>()?.base10_parse()?;
                    input.parse::<Token![;]>()?;
                }
                "method_sigs" => {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        let inner;
                        syn::parenthesized!(inner in content);
                        method_sigs.push(inner.parse::<syn::Signature>()?);
                        if !content.is_empty() {
                            let _ = content.parse::<Token![,]>();
                        }
                    }
                    input.parse::<Token![;]>()?;
                }
                "num_slots" => {
                    num_slots = Some(input.parse::<syn::LitInt>()?.base10_parse()?);
                    input.parse::<Token![;]>()?;
                }
                "methods" => {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        // Keep the braces — MetaMethod::parse expects them
                        methods.push(
                            content
                                .parse::<proc_macro2::TokenTree>()?
                                .into_token_stream(),
                        );
                        if !content.is_empty() {
                            let _ = content.parse::<Token![,]>();
                        }
                    }
                    input.parse::<Token![;]>()?;
                }
                "default_helpers" => {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        let name: syn::Ident = content.parse()?;
                        content.parse::<Token![=>]>()?;
                        let path_content;
                        syn::parenthesized!(path_content in content);
                        let path = path_content.parse::<proc_macro2::TokenStream>()?;
                        default_helpers.push((name, path));
                        if !content.is_empty() {
                            let _ = content.parse::<Token![,]>();
                        }
                    }
                    input.parse::<Token![;]>()?;
                }

                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown key `{other}` in __generate_vtable"),
                    ));
                }
            }
        }

        Ok(GenerateVtableInput {
            vtable_struct: vtable_struct.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "missing vtable_struct")
            })?,
            wrapper: wrapper.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "missing wrapper")
            })?,
            trait_path: trait_path.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "missing trait_path")
            })?,
            trait_generics: trait_generics.unwrap_or_default(),
            crate_path: crate_path.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "missing crate_path")
            })?,
            handles,
            values,
            num_slots: num_slots.ok_or_else(|| {
                syn::Error::new(proc_macro2::Span::call_site(), "missing num_slots")
            })?,
            own_method_count,
            methods,
            method_sigs,
            default_helpers,
        })
    }
}

/// Build the C function pointer signature for a vtable method:
/// `(param_types, return_type)`. Used for both vtable struct fields
/// and the trait impl's function pointer casts.
/// Returns `(param_types, return_type, ok_is_handle)`.
fn vtable_c_fn_sig(
    m: &crate::meta::MetaMethod,
    handle_names: &std::collections::HashSet<String>,
    value_names: &std::collections::HashSet<String>,
) -> (
    Vec<proc_macro2::TokenStream>,
    proc_macro2::TokenStream,
    bool,
) {
    let mut param_types = vec![quote! { *mut core::ffi::c_void }];
    for p in &m.params {
        match &p.kind {
            crate::meta::MetaParamKind::ImplTrait { .. } => {
                param_types.push(quote! { *mut core::ffi::c_void });
            }
            crate::meta::MetaParamKind::StrSlice => {
                param_types.push(quote! { *const ffier::FfierBytes });
                param_types.push(quote! { usize });
            }
            crate::meta::MetaParamKind::HandleSlice(_) => {
                param_types.push(quote! { *const *mut core::ffi::c_void });
                param_types.push(quote! { usize });
            }
            crate::meta::MetaParamKind::PrimitiveSlice(tp) => {
                let bt = &tp.bridge_type;
                param_types.push(quote! { *const <#bt as FfiType>::CRepr });
                param_types.push(quote! { usize });
            }
            crate::meta::MetaParamKind::Regular(tp) => {
                let bt = &tp.bridge_type;
                param_types.push(quote! { <#bt as FfiType>::CRepr });
            }
        }
    }

    let ok_is_handle = matches!(&m.ret, crate::meta::MetaReturn::Result { ok: Some(_), .. })
        && crate::meta::is_result_ok_handle(&m.rust_ret, handle_names);

    // `Result<Option<ValueStruct>, E>` writes presence and value separately.
    // Classification reads the Rust type; the emitted type comes from the
    // definition-site-resolved bridge type.
    let optional_value = match &m.ret {
        crate::meta::MetaReturn::Result { ok: Some(tp), .. }
            if crate::meta::option_value_struct_inner(&tp.rust_type, value_names).is_some() =>
        {
            crate::meta::option_inner_tokens(&tp.bridge_type)
        }
        _ => None,
    };

    let fn_ret = match &m.ret {
        crate::meta::MetaReturn::Void => quote! {},
        crate::meta::MetaReturn::Value(tp) => {
            if crate::meta::option_value_struct_inner(&tp.rust_type, value_names).is_some()
                && let Some(inner) = crate::meta::option_inner_tokens(&tp.bridge_type)
            {
                param_types.push(quote! { *mut <#inner as FfiType>::CRepr });
                quote! { -> bool }
            } else {
                let bt = &tp.bridge_type;
                quote! { -> <#bt as FfiType>::CRepr }
            }
        }
        crate::meta::MetaReturn::HandleSlice { .. } => {
            quote! { -> ffier::FfierObjectArray }
        }
        crate::meta::MetaReturn::Result { ok, .. } => {
            if let Some(inner) = &optional_value {
                param_types.push(quote! { *mut bool });
                param_types.push(quote! { *mut <#inner as FfiType>::CRepr });
                param_types.push(quote! { *mut *mut core::ffi::c_void });
                quote! { -> ffier::FfierResult }
            } else if ok_is_handle {
                param_types.push(quote! { *mut *mut core::ffi::c_void });
                quote! { -> *mut core::ffi::c_void }
            } else {
                if let Some(ok_tp) = ok {
                    let bt = &ok_tp.bridge_type;
                    param_types.push(quote! { *mut <#bt as FfiType>::CRepr });
                }
                param_types.push(quote! { *mut *mut core::ffi::c_void });
                quote! { -> ffier::FfierResult }
            }
        }
    };

    (param_types, fn_ret, ok_is_handle)
}

pub(crate) fn __generate_vtable(input: TokenStream) -> TokenStream {
    let inp = parse_macro_input!(input as GenerateVtableInput);
    let vtable_struct = &inp.vtable_struct;
    let wrapper = &inp.wrapper;
    let trait_path = &inp.trait_path;
    let trait_generics = &inp.trait_generics;
    let crate_path = &inp.crate_path;
    let handle_names: std::collections::HashSet<String> =
        inp.handles.iter().map(|h| h.to_string()).collect();
    let value_names: std::collections::HashSet<String> =
        inp.values.iter().map(|v| v.to_string()).collect();

    let default_helper_map: HashMap<String, &proc_macro2::TokenStream> = inp
        .default_helpers
        .iter()
        .map(|(name, path)| (name.to_string(), path))
        .collect();

    // Parse each method's metadata using the existing MetaMethod parser
    let parsed_methods: Vec<crate::meta::MetaMethod> = inp
        .methods
        .iter()
        .map(|tokens| {
            syn::parse2::<crate::meta::MetaMethod>(tokens.clone())
                .expect("failed to parse method metadata in __generate_vtable")
        })
        .collect();

    // --- Vtable struct fields ---
    let mut vtable_fields: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut next_slot = 0usize;

    // One slot may hold cfg-exclusive variants of a method: same name, same
    // index, disjoint predicates. Each variant gets its own guarded field, and
    // the complement of all variants keeps the slot filled with a reserved
    // pointer so later slots keep their offsets.
    let mut slot_methods: BTreeMap<usize, Vec<&crate::meta::MetaMethod>> = BTreeMap::new();
    for m in parsed_methods.iter().filter(|m| !m.raw_handle()) {
        slot_methods.entry(m.index()).or_default().push(m);
    }

    for (idx, variants) in &slot_methods {
        let idx = *idx;

        // Padding for gaps
        while next_slot < idx {
            let pad_name = format_ident!("__reserved_{next_slot}");
            vtable_fields.push(quote! {
                #[doc(hidden)]
                pub #pad_name: Option<unsafe extern "C" fn()>
            });
            next_slot += 1;
        }

        let mut negations: Vec<proc_macro2::TokenStream> = Vec::new();
        // An unconditional variant always fills the slot, so no padding is
        // needed alongside it.
        let mut slot_always_filled = false;
        for m in variants {
            let method_name = &m.name;
            let (param_types, fn_ret, _) = vtable_c_fn_sig(m, &handle_names, &value_names);
            let field = quote! {
                pub #method_name: Option<unsafe extern "C" fn(#(#param_types),*) #fn_ret>
            };
            // Each variant carries its own predicate. An unconditional variant
            // needs no guard; the others guard on their predicate alone, since
            // exclusive variants never coexist in one build.
            match m.cfg.as_ref() {
                Some(cfg) => {
                    let cfg_attr = m.cfg_attr();
                    vtable_fields.push(quote! {
                        #cfg_attr
                        #field
                    });
                    negations.push(quote!(not(#cfg)));
                }
                None => {
                    slot_always_filled = true;
                    vtable_fields.push(field);
                }
            }
        }

        if !slot_always_filled && !negations.is_empty() {
            let pad_name = format_ident!("__reserved_{idx}");
            let guard = quote!(all(#(#negations),*));
            vtable_fields.push(quote! {
                #[doc(hidden)]
                #[cfg(#guard)]
                pub #pad_name: Option<unsafe extern "C" fn()>
            });
        }

        next_slot = idx + 1;
    }

    // Pad remaining slots, including retired slots and raw-handle methods.
    while next_slot < inp.num_slots {
        let pad_name = format_ident!("__reserved_{next_slot}");
        vtable_fields.push(quote! {
            #[doc(hidden)]
            pub #pad_name: Option<unsafe extern "C" fn()>
        });
        next_slot += 1;
    }

    // --- Trait impl methods ---
    // Only process own methods (not supertrait methods) for the trait impl.

    // method_sigs are parallel to own non-raw methods (same filtering as @on_library_export).
    let own_methods = &parsed_methods[..inp.own_method_count];
    let own_non_raw: Vec<_> = own_methods.iter().filter(|m| !m.raw_handle()).collect();
    let own_method_impls: Vec<proc_macro2::TokenStream> = own_non_raw
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let method_name = &m.name;
            let slot = m.index() + 1;

            // Use the pre-baked trait method signature (preserves &mut impl PushStr etc.)
            let sig = &inp.method_sigs[i];

            let (fn_ptr_param_types, fn_ptr_ret, ok_is_handle) = vtable_c_fn_sig(m, &handle_names, &value_names);
            let fn_ptr_type = quote! {
                unsafe extern "C" fn(#(#fn_ptr_param_types),*) #fn_ptr_ret
            };

            // Build vtable call args (converting Rust values to C)
            let has_impl_trait_params = m.params.iter().any(|p| matches!(&p.kind, crate::meta::MetaParamKind::ImplTrait { .. }));

            let mut vtable_pre = Vec::<proc_macro2::TokenStream>::new();
            let mut vtable_args = Vec::<proc_macro2::TokenStream>::new();
            for p in &m.params {
                let id = &p.name;
                match &p.kind {
                    crate::meta::MetaParamKind::ImplTrait { .. } => {
                        vtable_args.push(quote! { core::ptr::null_mut() });
                    }
                    crate::meta::MetaParamKind::StrSlice => {
                        let vec_id = format_ident!("__{id}_ffierbytes");
                        vtable_pre.push(quote! {
                            let #vec_id: Vec<ffier::FfierBytes> = #id.iter()
                                .map(|s| unsafe { ffier::FfierBytes::from_str(s) })
                                .collect();
                        });
                        vtable_args.push(quote! { #vec_id.as_ptr() });
                        vtable_args.push(quote! { #vec_id.len() });
                    }
                    crate::meta::MetaParamKind::HandleSlice(_) => {
                        let vec_id = format_ident!("__{id}_handles");
                        vtable_pre.push(quote! {
                            let #vec_id: Vec<*mut core::ffi::c_void> = #id.iter()
                                .map(|item| FfiType::into_c(*item))
                                .collect();
                        });
                        vtable_args.push(quote! { #vec_id.as_ptr() });
                        vtable_args.push(quote! { #vec_id.len() });
                    }
                    crate::meta::MetaParamKind::PrimitiveSlice(_) => {
                        // Primitives are layout-compatible with their CRepr,
                        // so just pass the slice pointer and length directly.
                        vtable_args.push(quote! { #id.as_ptr() });
                        vtable_args.push(quote! { #id.len() });
                    }
                    crate::meta::MetaParamKind::Regular(_) => {
                        vtable_args.push(quote! { FfiType::into_c(#id) });
                    }
                }
            }

            // Build the call expression
            let raw_call = quote! { {
                #(#vtable_pre)*
                unsafe { __f(self.value.user_data as *mut core::ffi::c_void, #(#vtable_args),*) }
            } };

            // Build vtable_branch (converting C result back to Rust)
            let vtable_branch = if has_impl_trait_params {
                let wrapper_str = wrapper.to_string();
                let name_str = method_name.to_string();
                quote! {
                    let _ = __f;
                    panic!(
                        "{}: vtable dispatch for method `{}` with impl Trait params is not supported",
                        #wrapper_str, #name_str,
                    )
                }
            } else {
                match &m.ret {
                    crate::meta::MetaReturn::Void => raw_call.clone(),
                    crate::meta::MetaReturn::Value(tp) => {
                        if crate::meta::option_value_struct_inner(&tp.rust_type, &value_names)
                            .is_some()
                            && let Some(_inner) =
                                crate::meta::option_inner_tokens(&tp.bridge_type)
                        {
                            // Optional value return: the C side returns the
                            // presence flag and writes the value only when set.
                            let args = quote! { #(#vtable_args),* };
                            let sep = if vtable_args.is_empty() {
                                quote! {}
                            } else {
                                quote! {, }
                            };
                            quote! {{
                                let mut __out = core::mem::MaybeUninit::uninit();
                                let __is_some = unsafe { __f(
                                    self.value.user_data as *mut core::ffi::c_void,
                                    #args #sep __out.as_mut_ptr(),
                                ) };
                                if __is_some {
                                    Some(unsafe { FfiType::from_c(__out.assume_init()) })
                                } else {
                                    None
                                }
                            }}
                        } else {
                            quote! { unsafe { FfiType::from_c(#raw_call) } }
                        }
                    }
                    crate::meta::MetaReturn::HandleSlice { types: tp, .. } => {
                        // HandleSlice vtable dispatch: call __f which returns
                        // FfierObjectArray, reconstruct Vec<&T> from the handle array.
                        let bt = &tp.bridge_type;
                        quote! {{
                            let __arr = #raw_call;
                            if __arr.len == 0 {
                                Vec::new()
                            } else {
                                let __refs: Vec<&#bt> = (0..__arr.len)
                                    .map(|i| unsafe {
                                        let h = ffier::ffier_object_array_get(__arr, i);
                                        ffier::ffier_handle_borrow::<#bt>(h)
                                    })
                                    .collect();
                                unsafe { ffier::ffier_object_array_free(__arr) };
                                __refs
                            }
                        }}
                    }
                    crate::meta::MetaReturn::Result { ok, err_ident, .. } => {
                        let err_ty = format_ident!("{err_ident}");
                        let optional_value = ok.as_ref().and_then(|tp| {
                            crate::meta::option_value_struct_inner(&tp.rust_type, &value_names)
                                .and_then(|_| crate::meta::option_inner_tokens(&tp.bridge_type))
                        });
                        if let Some(inner) = optional_value {
                            // Result<Option<ValueStruct>, E>: presence flag plus
                            // a separate value out-param.
                            quote! {{
                                let mut __is_some = false;
                                let mut __out = core::mem::MaybeUninit::<<#inner as FfiType>::CRepr>::uninit();
                                let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
                                let __r = unsafe { __f(
                                    self.value.user_data as *mut core::ffi::c_void,
                                    #(#vtable_args,)*
                                    &mut __is_some as *mut bool,
                                    __out.as_mut_ptr(),
                                    &mut __err as *mut *mut core::ffi::c_void,
                                ) };
                                if __r == ffier::FFIER_RESULT_SUCCESS {
                                    if __is_some {
                                        Ok(Some(unsafe {
                                            <#inner as FfiType>::from_c(__out.assume_init())
                                        }))
                                    } else {
                                        Ok(None)
                                    }
                                } else {
                                    Err(unsafe { <#err_ty as FfiType>::from_c(__err) })
                                }
                            }}
                        } else if ok_is_handle {
                            // GLib-style: __f returns handle or null, err through out-param
                            let ok_conversion = match ok {
                                Some(_) => quote! { Ok(unsafe { FfiType::from_c(__raw) }) },
                                None => quote! { Ok(()) },
                            };
                            quote! {{
                                let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
                                let __raw = unsafe { __f(
                                    self.value.user_data as *mut core::ffi::c_void,
                                    #(#vtable_args,)*
                                    &mut __err as *mut *mut core::ffi::c_void,
                                ) };
                                if !__raw.is_null() {
                                    #ok_conversion
                                } else {
                                    Err(unsafe { <#err_ty as FfiType>::from_c(__err) })
                                }
                            }}
                        } else {
                            // OutParam: __f returns FfierResult, ok value through out-param
                            let (ok_decl, ok_conversion) = match ok {
                                Some(_) => (
                                    quote! { let mut __out = core::mem::MaybeUninit::uninit(); },
                                    quote! { Ok(unsafe { FfiType::from_c(__out.assume_init()) }) },
                                ),
                                None => (quote! {}, quote! { Ok(()) }),
                            };
                            let out_param = match ok {
                                Some(_) => quote! { __out.as_mut_ptr(), },
                                None => quote! {},
                            };
                            quote! {{
                                #ok_decl
                                let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
                                let __r = unsafe { __f(
                                    self.value.user_data as *mut core::ffi::c_void,
                                    #(#vtable_args,)*
                                    #out_param
                                    &mut __err as *mut *mut core::ffi::c_void,
                                ) };
                                if __r == ffier::FFIER_RESULT_SUCCESS {
                                    #ok_conversion
                                } else {
                                    Err(unsafe { <#err_ty as FfiType>::from_c(__err) })
                                }
                            }}
                        }
                    }
                }
            };

            // None branch: fallback or panic
            let name_str = method_name.to_string();
            // Only a method that actually has a default may fall back to one.
            // A required variant must never call the other variant's helper.
            let none_branch = match default_helper_map.get(&name_str) {
                Some(helper_path) if m.has_default() => {
                    let param_names: Vec<_> = m.params.iter().map(|p| &p.name).collect();
                    quote! { #helper_path(self #(, #param_names)*) }
                }
                _ => {
                    let wrapper_str = wrapper.to_string();
                    quote! {
                        panic!(
                            "{}: required vtable method `{}` not provided",
                            #wrapper_str, #name_str,
                        )
                    }
                }
            };

            // Full method body
            let lifetime_lint = m
                .receiver_lifetime
                .as_ref()
                .map(|_| quote! { #[allow(clippy::needless_lifetimes)] });
            // Cfg-exclusive variants share one slot but may differ in signature,
            // so each variant is gated exactly like its trait item.
            let cfg_attr = m.cfg_attr();
            quote! {
                #cfg_attr
                #lifetime_lint
                #sig {
                    let __field: Option<#fn_ptr_type> = unsafe {
                        self.value.field_or_none(#slot)
                    };
                    match __field {
                        Some(__f) => { #vtable_branch }
                        None => { #none_branch }
                    }
                }
            }
        })
        .collect();

    // --- Static assertions: verify proc-macro handle detection agrees with FfiType::IS_HANDLE ---
    let handle_assertions: Vec<proc_macro2::TokenStream> = own_non_raw
        .iter()
        .filter_map(|m| {
            if let crate::meta::MetaReturn::Result { ok: Some(tp), .. } = &m.ret {
                // Optional value returns carry an out-param pair, not a handle.
                if crate::meta::option_value_struct_inner(&tp.rust_type, &value_names).is_some() {
                    return None;
                }
                let bt = &tp.bridge_type;
                let ok_is_handle = crate::meta::is_result_ok_handle(&m.rust_ret, &handle_names);
                let method_name = m.name.to_string();
                // The assertion names the ok type, which may only exist in the
                // configurations where the method itself exists.
                let cfg_attr = m.cfg_attr();
                Some(quote! {
                    #cfg_attr
                    assert!(
                        <#bt as FfiType>::IS_HANDLE == #ok_is_handle,
                        concat!(
                            "handle detection mismatch for method `",
                            #method_name,
                            "`: proc-macro and FfiType::IS_HANDLE disagree",
                        ),
                    );
                })
            } else {
                None
            }
        })
        .collect();

    let output = quote! {
        #[repr(C)]
        pub struct #vtable_struct {
            /// Number of virtual method slots (excluding drop), including reserved gaps.
            pub _num_slots: u16,
            /// Reserved for future use; initialize to zero.
            pub _reserved1: u16,
            /// Reserved for future use; initialize to zero.
            pub _reserved2: u32,
            pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
            #(#vtable_fields,)*
        }

        const _: () = {
            use #crate_path::*;

            #(#handle_assertions)*

            impl #trait_path #trait_generics for #wrapper<'_> {
                #(#own_method_impls)*
            }

        };
    };

    output.into()
}
