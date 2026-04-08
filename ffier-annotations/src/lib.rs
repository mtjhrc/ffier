use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Data, DeriveInput, FnArg, GenericArgument, ImplItem, ItemImpl, ItemTrait, LitStr, Pat,
    PathArguments, ReturnType, Token, TraitItem, Type, parse::Parse, parse_macro_input,
    visit_mut::VisitMut,
};

// ---------------------------------------------------------------------------
// Type classification for params and return values
// ---------------------------------------------------------------------------

enum ParamKind {
    /// Uniform: bridge_type resolves via `<T as FfiType>::CRepr`.
    Regular(proc_macro2::TokenStream),
    /// `&[&str]` — slice of string references, expands to two C params.
    StrSlice,
    /// `impl Trait` parameter — generator resolves dispatch types from trait map.
    ImplTrait { trait_name: String, dispatch: String, passing: String },
}

enum ValueKind {
    Regular(proc_macro2::TokenStream),
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
    /// Method-level lifetime params (e.g. `['a, 'b]` from `fn foo<'a, 'b>(...)`).
    method_lifetimes: Vec<syn::Ident>,
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

fn classify_value(
    ty: &Type,
    reexport_types: &mut Vec<Type>,
    reexport_aliases: &mut Vec<syn::Ident>,
    alias_counter: &mut u32,
    helper_mod: &syn::Ident,
) -> ValueKind {
    ValueKind::Regular(bridge_tokens_for_type(
        ty,
        reexport_types,
        reexport_aliases,
        alias_counter,
        helper_mod,
    ))
}

// ---------------------------------------------------------------------------
// Main macro
// ---------------------------------------------------------------------------

#[proc_macro_attribute]
pub fn exportable(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);

    // Strip #[ffier(...)] attributes from methods before emitting the impl block
    let impl_block = {
        let mut block = input.clone();
        for item in &mut block.items {
            if let ImplItem::Fn(method) = item {
                method.attrs.retain(|a| !a.path().is_ident("ffier"));
                for arg in &mut method.sig.inputs {
                    if let FnArg::Typed(pat_ty) = arg {
                        pat_ty.attrs.retain(|a| !a.path().is_ident("ffier"));
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
    let self_ty_static = erase_lifetimes(self_ty);
    let struct_name = struct_ident.to_string();
    let struct_lower = camel_to_snake(&struct_name);

    let mut reexport_types: Vec<Type> = Vec::new();
    let mut reexport_aliases: Vec<syn::Ident> = Vec::new();
    let helper_mod_name = format_ident!("_ffier_{struct_lower}");

    let mut methods = Vec::new();
    let mut alias_counter = 0u32;
    let is_inherent = input.trait_.is_none();
    let mut warnings = Vec::new();

    for item in &input.items {
        let ImplItem::Fn(method) = item else { continue };
        let method_name = &method.sig.ident;
        let ffi_name_str = format!("{struct_lower}_{method_name}");

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
            param_idents.push(pat_ident.ident.clone());
            param_name_strs.push(pat_ident.ident.to_string());

            // Capture original type (Self-replaced, lifetimes preserved) for client codegen
            let param_ty_orig = replace_self_type(&pat_ty.ty, self_ty);
            param_orig_types.push(param_ty_orig);

            // Auto-detect `impl Trait` params — generator resolves dispatch types
            if let Some(trait_name) = extract_impl_trait_name(&pat_ty.ty) {
                let dispatch = parse_ffier_dispatch(&pat_ty.attrs)
                    .unwrap_or_else(|| "auto".to_string());
                param_kinds.push(ParamKind::ImplTrait {
                    trait_name, dispatch, passing: "value".to_string(),
                });
                continue;
            }

            // Detect `&dyn Trait`/`&mut dyn Trait` or `&F`/`&mut F` where F: Trait
            if let Some((trait_name, is_mut, is_dyn)) = extract_trait_ref(
                &pat_ty.ty, &method.sig.generics,
            ) {
                // &dyn Trait always uses dyn coerce (no concrete branching needed)
                let dispatch = if is_dyn {
                    "vtable".to_string()
                } else {
                    parse_ffier_dispatch(&pat_ty.attrs)
                        .unwrap_or_else(|| "auto".to_string())
                };
                let passing = if is_mut { "mut_ref" } else { "r#ref" };
                param_kinds.push(ParamKind::ImplTrait {
                    trait_name, dispatch, passing: passing.to_string(),
                });
                continue;
            }

            // Replace `Self` with the concrete (lifetime-erased) struct type
            let param_ty = replace_self_type(&pat_ty.ty, &self_ty_static);

            let kind = if is_str_slice(&param_ty) {
                ParamKind::StrSlice
            } else {
                ParamKind::Regular(bridge_tokens_for_type(
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
                    let ok_kind = if is_unit_type(&ok)
                        || (is_builder_return && is_self_return(&ok, &self_ty_static))
                    {
                        None
                    } else {
                        let vk = classify_value(
                            &ok,
                            &mut reexport_types,
                            &mut reexport_aliases,
                            &mut alias_counter,
                            &helper_mod_name,
                        );
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
                    ReturnKind::Value(vk)
                }
            }
        };

        let doc_lines = extract_doc_comments(&method.attrs);

        methods.push(MethodInfo {
            method_name: method_name.clone(),
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
                    **ty = replace_self_type(ty, self_ty);
                }
                out
            },
            method_lifetimes: method
                .sig
                .generics
                .lifetimes()
                .map(|lt| lt.lifetime.ident.clone())
                .collect(),
            doc_lines,
        });
    }

    // -----------------------------------------------------------------------
    // Metadata emission — structured tokens for generator proc macros
    // -----------------------------------------------------------------------


    let reexport_items: Vec<_> = reexport_types
        .iter()
        .zip(reexport_aliases.iter())
        .map(|(ty, alias)| {
            let erased = erase_lifetimes(ty);
            quote! { pub type #alias = super::#erased; }
        })
        .collect();

    let meta_macro_name = format_ident!("__ffier_meta_annotation_{struct_ident}");
    let check_const_name = format_ident!("__ffier_library_has_defined_{struct_ident}");

    // Build method metadata tokens
    let method_meta_tokens: Vec<_> = methods
        .iter()
        .map(|m| {
            let name = &m.method_name;
            let ffi_name_str = &m.ffi_name_str;
            let doc_tokens: Vec<_> = m.doc_lines.iter().map(|d| quote! { #d }).collect();

            let receiver_ident = if !m.has_receiver {
                format_ident!("none")
            } else if m.is_by_value {
                format_ident!("value")
            } else if m.is_mut {
                // "mut" is a keyword, so we use r#mut
                format_ident!("r#mut")
            } else {
                // "ref" is a keyword, so we use r#ref
                format_ident!("r#ref")
            };

            let is_builder = m.is_builder;

            let param_tokens: Vec<_> = m
                .param_idents
                .iter()
                .zip(m.param_kinds.iter())
                .zip(m.param_orig_types.iter())
                .map(|((id, kind), orig_ty)| {
                    let kind_tokens = emit_param_kind(kind);
                    quote! {
                        {
                            name = #id,
                            kind = #kind_tokens,
                            rust_type = (#orig_ty),
                        }
                    }
                })
                .collect();

            let ret_tokens = emit_return_kind(&m.ret);
            let rust_ret: proc_macro2::TokenStream = match &m.ret_orig_type {
                ReturnType::Default => quote! { () },
                ReturnType::Type(_, ty) => quote! { #ty },
            };

            // Method lifetime idents for metadata
            let method_lt_idents: Vec<_> = m
                .method_lifetimes
                .iter()
                .map(|lt| format_ident!("{}", lt))
                .collect();

            quote! {
                {
                    name = #name,
                    ffi_name = #ffi_name_str,
                    doc = [#(#doc_tokens),*],
                    receiver = #receiver_ident,
                    is_builder = #is_builder,
                    method_lifetimes = [#(#method_lt_idents),*],
                    params = [#(#param_tokens),*],
                    ret = #ret_tokens,
                    rust_ret = (#rust_ret),
                }
            }
        })
        .collect();

    // Build type alias metadata tokens
    let alias_meta_tokens: Vec<_> = reexport_aliases
        .iter()
        .zip(reexport_types.iter())
        .map(|(alias, _ty)| {
            // In the metadata, reference the alias via $crate::helper_mod::alias
            quote! { (#alias, $crate::#helper_mod_name::#alias) }
        })
        .collect();

    // Lifetime idents (without the tick) for metadata
    let lifetime_idents: Vec<_> = input
        .generics
        .lifetimes()
        .map(|lt| format_ident!("{}", lt.lifetime.ident))
        .collect();

    let struct_path_tokens = quote! { $crate::#struct_ident };

    let struct_name_lit = struct_name;

    let output = quote! {
        #impl_block

        impl ffier::FfiHandle for #self_ty_static {
            const C_HANDLE_NAME: &str = #struct_name_lit;
            fn as_handle(&self) -> *mut core::ffi::c_void {
                let value_offset = core::mem::offset_of!(
                    ffier::FfierTaggedBox<Self>, value
                );
                let box_ptr = (self as *const Self as *const u8)
                    .wrapping_sub(value_offset);
                box_ptr as *mut core::ffi::c_void
            }
        }

        impl ffier::FfiType for #self_ty_static {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &str = #struct_name_lit;
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

        /// If you see an error about this constant not being found in the crate root,
        /// add this type to your `ffier::library_definition!()` call in `lib.rs`.
        const _: () = crate::#check_const_name;

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #meta_macro_name {
            ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exportable,
                    name = #struct_ident,
                    struct_path = (#struct_path_tokens),
                    prefix = $prefix,
                    lifetimes = (#(#lifetime_idents),*),
                    type_aliases = [#(#alias_meta_tokens),*],
                    methods = [#(#method_meta_tokens),*],
                } $(, $($rest)*)? }
            };
        }
    };

    output.into()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

pub(crate) fn camel_to_snake(s: &str) -> String {
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

pub(crate) fn camel_to_upper_snake(s: &str) -> String {
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
            if let Type::Path(tp) = ty
                && tp.path.is_ident("Self")
            {
                *ty = self.0.clone();
                return;
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }
    let mut ty = ty.clone();
    Replacer(replacement).visit_type_mut(&mut ty);
    ty
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

/// Produce bridge_type tokens for any Rust type, including references.
///
/// This recursively handles reference and slice types, producing tokens
/// that resolve via `<T as FfiType>::CRepr` in the cdylib context:
/// - `&str` → `& str` (str is a keyword, always in scope)
/// - `&[u8]` → `& [u8]`
/// - `&Widget` → `& $crate::_ffier_mod::_Type0` (aliased for cross-crate)
/// - `i32` → `i32` (primitives emitted directly)
/// - `Widget` → `$crate::_ffier_mod::_Type0` (aliased)
fn bridge_tokens_for_type(
    ty: &Type,
    reexport_types: &mut Vec<Type>,
    reexport_aliases: &mut Vec<syn::Ident>,
    counter: &mut u32,
    helper_mod: &syn::Ident,
) -> proc_macro2::TokenStream {
    match ty {
        Type::Reference(ref_ty) => {
            let inner = bridge_tokens_for_type(
                &ref_ty.elem,
                reexport_types,
                reexport_aliases,
                counter,
                helper_mod,
            );
            // Use 'static lifetime so generated code like
            // `<&'static str as FfiType>::CRepr` doesn't trigger E0106.
            if ref_ty.mutability.is_some() {
                quote! { &'static mut #inner }
            } else {
                quote! { &'static #inner }
            }
        }
        Type::Slice(sl) => {
            let elem = bridge_tokens_for_type(
                &sl.elem,
                reexport_types,
                reexport_aliases,
                counter,
                helper_mod,
            );
            quote! { [#elem] }
        }
        // `str` is a keyword type — can't be aliased via `super::`, but
        // always resolves everywhere.
        Type::Path(tp) if tp.path.is_ident("str") => quote! { str },
        // Everything else goes through the standard aliasing.
        _ => type_tokens_for_macro(ty, reexport_types, reexport_aliases, counter, helper_mod),
    }
}

// ---------------------------------------------------------------------------
// Emission helpers — convert internal types to metadata tokens
// ---------------------------------------------------------------------------

fn emit_param_kind(k: &ParamKind) -> proc_macro2::TokenStream {
    match k {
        ParamKind::Regular(bridge_type) => {
            quote! { regular, bridge_type = (#bridge_type) }
        }
        ParamKind::StrSlice => quote! { str_slice },
        ParamKind::ImplTrait { trait_name, dispatch, passing } => {
            let dispatch_ident = format_ident!("{dispatch}");
            let passing_ident = format_ident!("{passing}");
            quote! { impl_trait, trait_name = #trait_name, dispatch = #dispatch_ident, passing = #passing_ident }
        }
    }
}

fn emit_value_kind(vk: &ValueKind) -> proc_macro2::TokenStream {
    match vk {
        ValueKind::Regular(bridge_type) => {
            quote! { regular, bridge_type = (#bridge_type), }
        }
    }
}

fn emit_return_kind(ret: &ReturnKind) -> proc_macro2::TokenStream {
    match ret {
        ReturnKind::Void => quote! { void },
        ReturnKind::Value(vk) => {
            let vk_tokens = emit_value_kind(vk);
            quote! { value(#vk_tokens) }
        }
        ReturnKind::Result {
            ok_ty,
            err_ty,
            err_ident,
        } => {
            let ok_tokens = match ok_ty {
                None => quote! { ok = void },
                Some(vk) => {
                    let vk_tokens = emit_value_kind(vk);
                    quote! { ok = some(#vk_tokens) }
                }
            };
            quote! { result(#ok_tokens, err_bridge_type = (#err_ty), err_ident = #err_ident,) }
        }
    }
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

    let name_str = name.to_string();
    let err_snake = camel_to_snake(&name_str);

    let meta_macro_name = format_ident!("__ffier_meta_annotation_{name}");
    let check_const_name = format_ident!("__ffier_library_has_defined_{name}");

    // Build variant metadata tokens
    let variant_meta_tokens: Vec<_> = data_enum
        .variants
        .iter()
        .map(|variant| {
            let var_ident = &variant.ident;
            let attrs = parse_ffier_variant_attrs(&variant.attrs).unwrap();
            let code = attrs.code;
            let message = attrs
                .message
                .unwrap_or_else(|| camel_to_human(&var_ident.to_string()));
            quote! {
                { name = #var_ident, code = #code, message = #message, }
            }
        })
        .collect();

    let error_path = quote! { $crate::#name };

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

        /// If you see an error about this constant not being found in the crate root,
        /// add this type to your `ffier::library_definition!()` call in `lib.rs`.
        const _: () = crate::#check_const_name;

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #meta_macro_name {
            ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @error,
                    name = #name,
                    path = (#error_path),
                    prefix = $prefix,
                    variants = [#(#variant_meta_tokens),*],
                } $(, $($rest)*)? }
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

/// Check if an attribute is `#[ffier(skip)]`.
fn is_ffier_skip(attr: &syn::Attribute) -> bool {
    if !attr.path().is_ident("ffier") {
        return false;
    }
    let mut found = false;
    let _ = attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("skip") {
            found = true;
        }
        Ok(())
    });
    found
}

/// Parse `#[ffier(dispatch = concrete)]` or `#[ffier(dispatch = vtable)]` from method attrs.
fn parse_ffier_dispatch(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("ffier") {
            continue;
        }
        let mut result = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("dispatch") {
                let value = meta.value()?;
                let mode: syn::Ident = value.parse()?;
                result = Some(mode.to_string());
            }
            Ok(())
        });
        if result.is_some() {
            return result;
        }
    }
    None
}

/// Detect trait-dispatched reference params:
/// - `&F` / `&mut F` where `F: Trait` (generic with trait bound)
/// - `&dyn Trait` / `&mut dyn Trait` (trait object reference)
///
/// Returns `(trait_name, is_mut, is_dyn)`.
fn extract_trait_ref(
    ty: &Type,
    generics: &syn::Generics,
) -> Option<(String, bool, bool)> {
    let Type::Reference(ref_ty) = ty else { return None };
    let is_mut = ref_ty.mutability.is_some();

    // &impl Trait / &mut impl Trait
    if let Type::ImplTrait(impl_trait) = &*ref_ty.elem {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(tb) = bound {
                if let Some(seg) = tb.path.segments.last() {
                    return Some((seg.ident.to_string(), is_mut, false));
                }
            }
        }
    }

    // &dyn Trait / &mut dyn Trait
    if let Type::TraitObject(to) = &*ref_ty.elem {
        for bound in &to.bounds {
            if let syn::TypeParamBound::Trait(tb) = bound {
                if let Some(seg) = tb.path.segments.last() {
                    return Some((seg.ident.to_string(), is_mut, true));
                }
            }
        }
    }

    // &F / &mut F where F: Trait
    if let Type::Path(tp) = &*ref_ty.elem {
        if tp.path.segments.len() != 1 { return None; }
        let param_name = tp.path.segments[0].ident.to_string();
        for param in generics.type_params() {
            if param.ident != param_name { continue; }
            for bound in &param.bounds {
                if let syn::TypeParamBound::Trait(tb) = bound {
                    if let Some(seg) = tb.path.segments.last() {
                        return Some((seg.ident.to_string(), is_mut, false));
                    }
                }
            }
        }
    }

    None
}

/// Extract the trait name from an `impl Trait` type.
fn extract_impl_trait_name(ty: &Type) -> Option<String> {
    if let Type::ImplTrait(impl_trait) = ty {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(trait_bound) = bound {
                if let Some(seg) = trait_bound.path.segments.last() {
                    return Some(seg.ident.to_string());
                }
            }
        }
    }
    None
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

// ===========================================================================
// #[ffier::implementable] — C users can implement a Rust trait via vtable
// ===========================================================================

struct ImplementableArgs {
    _prefix: Option<String>,
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

        Ok(Self {
            _prefix: prefix,
            supers,
        })
    }
}

/// A method signature extracted from a trait for vtable generation.
struct VtableMethod {
    name: syn::Ident,
    trait_name: Option<syn::Ident>,
    params: Vec<VtableMethodParam>,
    /// bridge_type for return (None = void)
    ret_bridge_type: Option<proc_macro2::TokenStream>,
    /// rust_type for return (None = void)
    ret_rust_type: Option<proc_macro2::TokenStream>,
}

struct VtableMethodParam {
    ident: syn::Ident,
    bridge_type: proc_macro2::TokenStream,
    rust_type: proc_macro2::TokenStream,
}

fn extract_vtable_methods(
    trait_item: &ItemTrait,
    supers: &[SupertraitBlock],
    reexport_types: &mut Vec<Type>,
    reexport_aliases: &mut Vec<syn::Ident>,
    alias_counter: &mut u32,
    helper_mod: &syn::Ident,
) -> Vec<VtableMethod> {
    let mut methods = Vec::new();

    for item in &trait_item.items {
        let TraitItem::Fn(method) = item else { continue };
        if let Some(vm) = parse_trait_method_sig(
            &method.sig, None,
            reexport_types, reexport_aliases, alias_counter, helper_mod,
        ) {
            methods.push(vm);
        }
    }

    for sup in supers {
        for method in &sup.methods {
            if let Some(vm) = parse_trait_method_sig(
                &method.sig, Some(sup.trait_name.clone()),
                reexport_types, reexport_aliases, alias_counter, helper_mod,
            ) {
                methods.push(vm);
            }
        }
    }

    methods
}

fn parse_trait_method_sig(
    sig: &syn::Signature,
    trait_name: Option<syn::Ident>,
    reexport_types: &mut Vec<Type>,
    reexport_aliases: &mut Vec<syn::Ident>,
    alias_counter: &mut u32,
    helper_mod: &syn::Ident,
) -> Option<VtableMethod> {
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
            let Pat::Ident(pi) = &*pt.pat else { return None };
            let bridge_type = bridge_tokens_for_type(
                &pt.ty, reexport_types, reexport_aliases, alias_counter, helper_mod,
            );
            let erased = erase_lifetimes(&pt.ty);
            let rust_type = quote! { #erased };
            Some(VtableMethodParam {
                ident: pi.ident.clone(),
                bridge_type,
                rust_type,
            })
        })
        .collect();

    let (ret_bridge_type, ret_rust_type) = match &sig.output {
        ReturnType::Default => (None, None),
        ReturnType::Type(_, ty) => {
            let bt = bridge_tokens_for_type(
                ty, reexport_types, reexport_aliases, alias_counter, helper_mod,
            );
            let erased = erase_lifetimes(ty);
            (Some(bt), Some(quote! { #erased }))
        }
    };

    Some(VtableMethod {
        name: sig.ident.clone(),
        trait_name,
        params,
        ret_bridge_type,
        ret_rust_type,
    })
}

/// Generate `impl Trait for FfierBoxDyn<dyn Trait>` for dynamic dispatch fallback.
///
/// This enables the generator to wrap concrete handles into `FfierBoxDyn`
/// when the combinatorial dispatch exceeds the branch limit.
///
/// `#[ffier::implementable]` implies `#[ffier::dispatch]` — use this
/// annotation alone when you want dynamic dispatch fallback without
/// exporting the trait's vtable to C.
#[proc_macro_attribute]
pub fn dispatch(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let trait_item = parse_macro_input!(item as ItemTrait);
    let original_trait = trait_item.clone();
    let trait_name = &trait_item.ident;

    let method_impls: Vec<_> = trait_item
        .items
        .iter()
        .filter_map(|item| {
            let TraitItem::Fn(method) = item else { return None };
            let sig = &method.sig;
            // Must have &self receiver
            if !matches!(sig.inputs.first(), Some(FnArg::Receiver(_))) {
                return None;
            }
            let name = &sig.ident;
            let params: Vec<_> = sig.inputs.iter().skip(1).filter_map(|arg| {
                let FnArg::Typed(pt) = arg else { return None };
                let Pat::Ident(pi) = &*pt.pat else { return None };
                Some(pi.ident.clone())
            }).collect();
            let call = quote! { self.0.#name(#(#params),*) };
            Some(quote! { #sig { #call } })
        })
        .collect();

    let output = quote! {
        #original_trait

        impl #trait_name for ffier::FfierBoxDyn<dyn #trait_name> {
            #(#method_impls)*
        }
    };

    output.into()
}

#[proc_macro_attribute]
pub fn implementable(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ImplementableArgs);
    let trait_item = parse_macro_input!(item as ItemTrait);
    let original_trait = trait_item.clone();

    let trait_name = &trait_item.ident;
    let trait_name_str = trait_name.to_string();
    let trait_snake = camel_to_snake(&trait_name_str);

    let vtable_struct_name = format_ident!("{trait_name_str}Vtable");
    let wrapper_name = format_ident!("Vtable{trait_name_str}");
    let wrapper_c_handle_suffix = format!("Vtable{trait_name_str}");

    // Reexport state for bridge_tokens_for_type
    let helper_mod_name = format_ident!("_ffier_vtable_{trait_snake}");
    let mut reexport_types: Vec<Type> = Vec::new();
    let mut reexport_aliases: Vec<syn::Ident> = Vec::new();
    let mut alias_counter = 0u32;

    // Extract all methods (trait + supertraits)
    let vtable_methods = extract_vtable_methods(
        &trait_item, &args.supers,
        &mut reexport_types, &mut reexport_aliases, &mut alias_counter, &helper_mod_name,
    );

    // --- Generate vtable struct fields ---
    let vtable_fields: Vec<_> = vtable_methods
        .iter()
        .map(|m| {
            let name = &m.name;
            let params: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let bt = &p.bridge_type;
                    quote! { <#bt as ffier::FfiType>::CRepr }
                })
                .collect();
            let ret = match &m.ret_bridge_type {
                None => quote! {},
                Some(bt) => quote! { -> <#bt as ffier::FfiType>::CRepr },
            };
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

    // Helper: generate vtable call-through body for a method
    let vtable_call_body = |vm: &VtableMethod, sig: &syn::Signature| -> proc_macro2::TokenStream {
        let name = &vm.name;
        let vtable_args: Vec<_> = vm
            .params
            .iter()
            .map(|p| {
                let id = &p.ident;
                let bt = &p.bridge_type;
                quote! { <#bt as ffier::FfiType>::into_c(#id) }
            })
            .collect();
        let call = quote! {
            unsafe { ((*self.vtable).#name)(self.user_data, #(#vtable_args),*) }
        };
        let body = match &vm.ret_bridge_type {
            None => call,
            Some(bt) => quote! { <#bt as ffier::FfiType>::from_c(#call) },
        };
        quote! { #sig { #body } }
    };

    let own_method_impls: Vec<_> = trait_item_erased
        .items
        .iter()
        .filter_map(|item| {
            let TraitItem::Fn(method) = item else { return None };
            let name = &method.sig.ident;
            let vm = vtable_methods.iter().find(|v| v.name == *name)?;
            Some(vtable_call_body(vm, &method.sig))
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
                    Some(vtable_call_body(vm, &method.sig))
                })
                .collect();

            quote! {
                impl #tn for #wrapper_name {
                    #(#method_impls)*
                }
            }
        })
        .collect();

    // --- Metadata emission ---
    let meta_macro_name = format_ident!("__ffier_meta_annotation_{trait_name}");
    let check_const_name = format_ident!("__ffier_library_has_defined_{trait_name}");

    // Build vtable field metadata — currently no extra data fields are supported,
    // so this is always empty. Method function pointer fields are handled by
    // vtable_methods metadata instead.
    let vtable_field_meta: Vec<proc_macro2::TokenStream> = Vec::new();

    // Build vtable method metadata
    let vtable_method_meta: Vec<_> = vtable_methods
        .iter()
        .map(|m| {
            let mname = &m.name;
            let param_meta: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let id = &p.ident;
                    let bt = &p.bridge_type;
                    let rt = &p.rust_type;
                    quote! { { name = #id, bridge_type = (#bt), rust_type = (#rt), } }
                })
                .collect();
            let ret = match (&m.ret_bridge_type, &m.ret_rust_type) {
                (None, _) => quote! { void },
                (Some(bt), Some(rt)) => quote! { value(bridge_type = (#bt), rust_type = (#rt),) },
                _ => unreachable!(),
            };
            quote! {
                { name = #mname, params = [#(#param_meta),*], ret = #ret, }
            }
        })
        .collect();

    let trait_path_tokens = quote! { $crate::#trait_name };

    let reexport_items: Vec<_> = reexport_types
        .iter()
        .zip(reexport_aliases.iter())
        .map(|(ty, alias)| {
            let erased = erase_lifetimes(ty);
            quote! { pub type #alias = super::#erased; }
        })
        .collect();

    // Generate FfierBoxDyn delegation (implies #[ffier::dispatch])
    // For traits with supertraits, we also need to delegate the supertrait methods.
    let has_supertraits = !args.supers.is_empty()
        || trait_item.supertraits.iter().any(|b| matches!(b, syn::TypeParamBound::Trait(_)));

    let boxdyn_impl = if !has_supertraits {
        let boxdyn_method_impls: Vec<_> = trait_item
            .items
            .iter()
            .filter_map(|item| {
                let TraitItem::Fn(method) = item else { return None };
                let sig = &method.sig;
                if !matches!(sig.inputs.first(), Some(FnArg::Receiver(_))) {
                    return None;
                }
                let name = &sig.ident;
                let params: Vec<_> = sig.inputs.iter().skip(1).filter_map(|arg| {
                    let FnArg::Typed(pt) = arg else { return None };
                    let Pat::Ident(pi) = &*pt.pat else { return None };
                    Some(pi.ident.clone())
                }).collect();
                Some(quote! { #sig { self.0.#name(#(#params),*) } })
            })
            .collect();

        quote! {
            impl #trait_name for ffier::FfierBoxDyn<dyn #trait_name> {
                #(#boxdyn_method_impls)*
            }
        }
    } else {
        // TODO: generate supertrait delegation for FfierBoxDyn
        quote! {}
    };

    let output = quote! {
        #original_trait

        #boxdyn_impl

        #[doc(hidden)]
        pub mod #helper_mod_name {
            #(#reexport_items)*
        }

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
            const C_HANDLE_NAME: &str = #wrapper_c_handle_suffix;
            fn as_handle(&self) -> *mut core::ffi::c_void {
                let value_offset = core::mem::offset_of!(
                    ffier::FfierTaggedBox<Self>, value
                );
                let box_ptr = (self as *const Self as *const u8)
                    .wrapping_sub(value_offset);
                box_ptr as *mut core::ffi::c_void
            }
        }

        impl ffier::FfiType for #wrapper_name {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &str = #wrapper_c_handle_suffix;
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

        /// If you see an error about this constant not being found in the crate root,
        /// add this type to your `ffier::library_definition!()` call in `lib.rs`.
        const _: () = crate::#check_const_name;

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #meta_macro_name {
            ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @implementable,
                    trait_name = #trait_name,
                    trait_path = (#trait_path_tokens),
                    prefix = $prefix,
                    vtable_struct = ($crate::#vtable_struct_name),
                    wrapper_name = ($crate::#wrapper_name),
                    vtable_fields = [#(#vtable_field_meta),*],
                    vtable_methods = [#(#vtable_method_meta),*],
                } $(, $($rest)*)? }
            };
        }
    };

    output.into()
}

// ===========================================================================
// #[ffier::trait_impl] — export trait method impls as C functions
// ===========================================================================

#[proc_macro_attribute]
pub fn trait_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);

    // Build the output impl block with #[ffier(skip)] attributes stripped.
    let mut clean_impl = input.clone();
    for item in &mut clean_impl.items {
        if let ImplItem::Fn(method) = item {
            method.attrs.retain(|a| !is_ffier_skip(a));
        }
    }

    // Extract trait path and name
    let Some((_, trait_path, _)) = &input.trait_ else {
        return syn::Error::new_spanned(&input, "trait_impl requires a trait impl block")
            .to_compile_error()
            .into();
    };
    let trait_last_seg = trait_path
        .segments
        .last()
        .expect("trait path must have segments");
    let trait_name = trait_last_seg.ident.clone();
    let trait_snake = camel_to_snake(&trait_name.to_string());

    // Extract lifetime arguments from the trait path (e.g. 'static from AttachDevice<'static>,
    // or 'a from AttachDevice<'a>). These may differ from the impl's declared generics.
    let trait_lt_args: Vec<String> = match &trait_last_seg.arguments {
        syn::PathArguments::AngleBracketed(ab) => ab
            .args
            .iter()
            .filter_map(|arg| {
                if let syn::GenericArgument::Lifetime(lt) = arg {
                    Some(lt.ident.to_string())
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    };

    // Extract struct type
    let Type::Path(ref struct_type_path) = *input.self_ty else {
        return syn::Error::new_spanned(&input.self_ty, "expected a named struct type")
            .to_compile_error()
            .into();
    };
    let struct_ident = &struct_type_path
        .path
        .segments
        .last()
        .expect("expected struct name")
        .ident;
    let struct_name = struct_ident.to_string();
    let struct_snake = camel_to_snake(&struct_name);

    // Reexport state for bridge_tokens_for_type
    let helper_mod_name = format_ident!("_ffier_impl_{trait_snake}_for_{struct_snake}");
    let mut reexport_types: Vec<Type> = Vec::new();
    let mut reexport_aliases: Vec<syn::Ident> = Vec::new();
    let mut alias_counter = 0u32;

    // Extract methods, skipping any marked with #[ffier(skip)].
    let methods: Vec<VtableMethod> = input
        .items
        .iter()
        .filter_map(|item| {
            let ImplItem::Fn(method) = item else {
                return None;
            };
            if method.attrs.iter().any(|a| is_ffier_skip(a)) {
                return None;
            }
            parse_trait_method_sig(
                &method.sig, None,
                &mut reexport_types, &mut reexport_aliases, &mut alias_counter, &helper_mod_name,
            )
        })
        .collect();

    let reexport_items: Vec<_> = reexport_types
        .iter()
        .zip(reexport_aliases.iter())
        .map(|(ty, alias)| {
            let erased = erase_lifetimes(ty);
            quote! { pub type #alias = super::#erased; }
        })
        .collect();

    // Build method metadata tokens (same format as implementable vtable_methods)
    let method_meta: Vec<_> = methods
        .iter()
        .map(|m| {
            let mname = &m.name;
            let param_meta: Vec<_> = m
                .params
                .iter()
                .map(|p| {
                    let id = &p.ident;
                    let bt = &p.bridge_type;
                    let rt = &p.rust_type;
                    quote! { { name = #id, bridge_type = (#bt), rust_type = (#rt), } }
                })
                .collect();
            let ret = match (&m.ret_bridge_type, &m.ret_rust_type) {
                (None, _) => quote! { void },
                (Some(bt), Some(rt)) => quote! { value(bridge_type = (#bt), rust_type = (#rt),) },
                _ => unreachable!(),
            };
            quote! {
                { name = #mname, params = [#(#param_meta),*], ret = #ret, }
            }
        })
        .collect();

    let lifetime_idents: Vec<_> = input
        .generics
        .lifetimes()
        .map(|lt| format_ident!("{}", lt.lifetime.ident))
        .collect();

    let meta_macro_name = format_ident!("__ffier_meta_annotation_{trait_name}_for_{struct_ident}");
    let check_const_name = format_ident!("__ffier_library_has_defined_{trait_name}_for_{struct_ident}");
    let struct_path_tokens = quote! { $crate::#struct_ident };
    let trait_path_tokens = quote! { $crate::#trait_name };

    let output = quote! {
        #clean_impl

        #[doc(hidden)]
        pub mod #helper_mod_name {
            #(#reexport_items)*
        }

        /// If you see an error about this constant not being found in the crate root,
        /// add this type to your `ffier::library_definition!()` call in `lib.rs`.
        const _: () = crate::#check_const_name;

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #meta_macro_name {
            ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @trait_impl,
                    trait_name = #trait_name,
                    struct_name = #struct_ident,
                    struct_path = (#struct_path_tokens),
                    trait_path = (#trait_path_tokens),
                    prefix = $prefix,
                    lifetimes = (#(#lifetime_idents),*),
                    trait_lifetime_args = [#(#trait_lt_args),*],
                    methods = [#(#method_meta),*],
                } $(, $($rest)*)? }
            };
        }
    };

    output.into()
}

// ===========================================================================
// ffier::library_definition! — define a library's exported types
// ===========================================================================

/// Define the list of exported types for a library.
///
/// ```ignore
/// ffier::library_definition!("mylib",
///     Calculator, CalcError,
///     TextBuffer, BufferError,
/// );
///
/// // In cdylib:
/// mylib::__ffier_mylib_library!(ffier_gen_c_macros::generate);
/// ```
///
/// Supports three entry kinds:
/// - `TypeName` — exportable struct or error enum
/// - `trait TraitName` — implementable trait
/// - `TraitName for StructName` — trait impl bridge
#[proc_macro]
pub fn library_definition(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as LibraryInput);
    let prefix_lit = &parsed.prefix;
    let prefix_str = parsed.prefix.value();

    let mut macro_idents: Vec<syn::Ident> = Vec::new();
    let mut check_consts: Vec<syn::Ident> = Vec::new();

    for entry in &parsed.entries {
        let (macro_name, const_name) = match entry {
            LibraryEntry::Plain(name) => (
                format_ident!("__ffier_meta_annotation_{name}"),
                format_ident!("__ffier_library_has_defined_{name}"),
            ),
            LibraryEntry::Trait(name) => (
                format_ident!("__ffier_meta_annotation_{name}"),
                format_ident!("__ffier_library_has_defined_{name}"),
            ),
            LibraryEntry::TraitImpl {
                trait_name,
                struct_name,
            } => (
                format_ident!("__ffier_meta_annotation_{trait_name}_for_{struct_name}"),
                format_ident!("__ffier_library_has_defined_{trait_name}_for_{struct_name}"),
            ),
        };
        macro_idents.push(macro_name);
        check_consts.push(const_name);
    }

    if macro_idents.is_empty() {
        return quote! { compile_error!("library_definition! requires at least one type"); }
            .into();
    }

    let first = &macro_idents[0];
    let rest = &macro_idents[1..];
    let rest_paths: Vec<proc_macro2::TokenStream> = rest
        .iter()
        .map(|id| quote! { $crate::#id })
        .collect();

    let entry_macro_name = format_ident!("__ffier_{prefix_str}_library");

    let output = quote! {
        #(
            #[doc(hidden)]
            pub const #check_consts: () = ();
        )*

        #[doc(hidden)]
        #[macro_export]
        macro_rules! __ffier_chain {
            // Recursive: append metadata, expand next
            ({ $($meta:tt)* }, $prefix:literal, $final_cb:path,
             [$($acc:tt)*], [$next:path, $($remaining:path),*]) => {
                $next! { $prefix, $crate::__ffier_chain,
                    $prefix, $final_cb,
                    [$($acc)* { $($meta)* }],
                    [$($remaining),*]
                }
            };
            // Recursive: append metadata, expand last item
            ({ $($meta:tt)* }, $prefix:literal, $final_cb:path,
             [$($acc:tt)*], [$next:path]) => {
                $next! { $prefix, $crate::__ffier_chain,
                    $prefix, $final_cb,
                    [$($acc)* { $($meta)* }],
                    []
                }
            };
            // Base case: call the final callback with everything
            ({ $($meta:tt)* }, $prefix:literal, $final_cb:path,
             [$($acc:tt)*], []) => {
                $final_cb! { $($acc)* { $($meta)* } }
            };
        }

        #[macro_export]
        macro_rules! #entry_macro_name {
            ($callback:path) => {
                $crate::#first! { #prefix_lit, $crate::__ffier_chain,
                    #prefix_lit, $callback,
                    [],
                    [#(#rest_paths),*]
                }
            };
        }
    };

    output.into()
}

struct LibraryInput {
    prefix: LitStr,
    entries: Vec<LibraryEntry>,
}

enum LibraryEntry {
    Plain(syn::Ident),
    Trait(syn::Ident),
    TraitImpl {
        trait_name: syn::Ident,
        struct_name: syn::Ident,
    },
}

impl Parse for LibraryInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let prefix: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;

        let mut entries = Vec::new();
        while !input.is_empty() {
            if input.peek(Token![trait]) {
                // `trait TraitName`
                input.parse::<Token![trait]>()?;
                let name: syn::Ident = input.parse()?;
                entries.push(LibraryEntry::Trait(name));
            } else {
                let first: syn::Ident = input.parse()?;
                if input.peek(Token![for]) {
                    // `TraitName for StructName`
                    input.parse::<Token![for]>()?;
                    let second: syn::Ident = input.parse()?;
                    entries.push(LibraryEntry::TraitImpl {
                        trait_name: first,
                        struct_name: second,
                    });
                } else {
                    // Plain type name
                    entries.push(LibraryEntry::Plain(first));
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(LibraryInput { prefix, entries })
    }
}
