use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Data, DeriveInput, FnArg, GenericArgument, ImplItem, ItemImpl, ItemTrait, ItemUse, LitStr, Pat,
    PathArguments, ReturnType, Token, TraitItem, Type, parse::Parse, parse_macro_input,
    visit_mut::VisitMut,
};

use ffier_meta::{camel_to_snake, camel_to_upper_snake, erase_lifetimes};

/// Counter for generating unique `#[macro_export]` macro names.
/// The exported name is an implementation detail — users access the macro
/// only through the `pub use ... as __ffier_meta_*` alias placed next to the type.
static MACRO_COUNTER: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Type classification for params and return values
// ---------------------------------------------------------------------------

enum ParamKind {
    /// Uniform: bridge_type resolves via `<T as FfiType>::CRepr`.
    Regular(proc_macro2::TokenStream),
    /// `&[&str]` — slice of string references, expands to two C params.
    StrSlice,
    /// `impl Trait` parameter — generator resolves dispatch types from trait map.
    ImplTrait {
        trait_name: String,
        dispatch: String,
    },
}

enum ReturnKind {
    Void,
    Value(proc_macro2::TokenStream),
    Result {
        ok_ty: Option<proc_macro2::TokenStream>,
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

/// Emit `impl FfiHandle for T` and `impl FfiType for T` blocks for an exported type.
///
/// `ty` is the type (e.g. `Widget` or `VtableFruit`), `c_handle_name` is the
/// C handle typedef name string (e.g. `"Widget"`, `"VtableFruit"`).
/// Emit `impl FfiHandle` and `impl FfiType` for a handle type.
///
/// `tag_const` is a token stream referencing the type tag constant, e.g.
/// `crate::__ffier_type_tag_Widget`. The actual value is provided by
/// `library_definition!` which emits `pub const __ffier_type_tag_Widget: u32 = N;`.
fn emit_ffi_handle_impls(
    ty: &proc_macro2::TokenStream,
    c_handle_name: &str,
    tag_const: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    quote! {
        impl ffier::FfiHandle for #ty {
            const C_HANDLE_NAME: &str = #c_handle_name;
            const TYPE_TAG: u32 = #tag_const;
            fn as_handle(&self) -> *mut core::ffi::c_void {
                let value_offset = core::mem::offset_of!(
                    ffier::FfierTaggedBox<Self>, value
                );
                let box_ptr = (self as *const Self as *const u8)
                    .wrapping_sub(value_offset);
                box_ptr as *mut core::ffi::c_void
            }
        }

        impl ffier::FfiType for #ty {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &str = #c_handle_name;
            fn into_c(self) -> *mut core::ffi::c_void {
                let tagged = ffier::FfierTaggedBox {
                    type_tag: #tag_const,
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
    }
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

    let helper_mod_name = format_ident!("_ffier_{struct_lower}");
    let mut ctx = AliasContext::new(helper_mod_name.clone());

    let mut methods = Vec::new();
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
        let mut param_kinds = Vec::new();
        let mut param_orig_types = Vec::new();

        let skip_n = if has_receiver { 1 } else { 0 };
        for arg in method.sig.inputs.iter().skip(skip_n) {
            let FnArg::Typed(pat_ty) = arg else { continue };
            let Pat::Ident(pat_ident) = &*pat_ty.pat else {
                continue;
            };
            param_idents.push(pat_ident.ident.clone());

            // Capture original type (Self-replaced, lifetimes preserved) for client codegen
            let param_ty_orig = replace_self_type(&pat_ty.ty, self_ty);
            param_orig_types.push(param_ty_orig);

            // Auto-detect `impl Trait` params — generator resolves dispatch types
            if let Some(trait_name) = extract_impl_trait_name(&pat_ty.ty) {
                let dispatch =
                    parse_ffier_dispatch(&pat_ty.attrs).unwrap_or_else(|| "auto".to_string());
                param_kinds.push(ParamKind::ImplTrait {
                    trait_name,
                    dispatch,
                });
                continue;
            }

            // Replace `Self` with the concrete (lifetime-erased) struct type
            let param_ty = replace_self_type(&pat_ty.ty, &self_ty_static);

            let kind = if is_str_slice(&param_ty) {
                ParamKind::StrSlice
            } else {
                ParamKind::Regular(ctx.bridge_tokens(&param_ty))
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
                    // Result<Self, E> in builder context → treat as Result<(), E> for C
                    let ok_kind = if is_unit_type(&ok)
                        || (is_builder_return && is_self_return(&ok, &self_ty_static))
                    {
                        None
                    } else {
                        Some(ctx.bridge_tokens(&ok))
                    };
                    ReturnKind::Result {
                        ok_ty: ok_kind,
                        err_ident,
                    }
                } else {
                    ReturnKind::Value(ctx.bridge_tokens(ty))
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

    let reexport_items_crate = ctx.reexport_items_crate();

    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{struct_lower}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{struct_ident}");

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

    // Lifetime idents (without the tick) for metadata
    let lifetime_idents: Vec<_> = input
        .generics
        .lifetimes()
        .map(|lt| format_ident!("{}", lt.lifetime.ident))
        .collect();

    let struct_path_tokens = quote! { $crate::#struct_ident };

    let struct_name_lit = struct_name;
    let tag_const_name = format_ident!("__ffier_type_tag_{struct_ident}");
    let tag_const = quote! { crate::#tag_const_name };
    let ffi_handle_impls = emit_ffi_handle_impls(&quote! { #self_ty_static }, &struct_name_lit, &tag_const);

    let output = quote! {
        #impl_block

        #ffi_handle_impls

        #(#warnings)*

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            (@reexport) => {
                #[doc(hidden)]
                pub mod #helper_mod_name {
                    #(#reexport_items_crate)*
                }
            };
            // Tagged invocation (from library_definition! shim): includes type_tag
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exportable,
                    name = #struct_ident,
                    struct_path = (#struct_path_tokens),
                    prefix = $prefix,
                    type_tag = $type_tag,
                    lifetimes = (#(#lifetime_idents),*),
                    methods = [#(#method_meta_tokens),*],
                } $(, $($rest)*)? }
            };
            // Untagged invocation (legacy / direct): type_tag defaults to 0
            ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @exportable,
                    name = #struct_ident,
                    struct_path = (#struct_path_tokens),
                    prefix = $prefix,
                    type_tag = 0,
                    lifetimes = (#(#lifetime_idents),*),
                    methods = [#(#method_meta_tokens),*],
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
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

/// Tracks type aliases needed for cross-crate `$crate::` resolution in metadata macros.
struct AliasContext {
    types: Vec<Type>,
    aliases: Vec<syn::Ident>,
    counter: u32,
    helper_mod: syn::Ident,
}

const PRIMITIVES: &[&str] = &[
    "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "isize", "usize", "bool", "f32", "f64",
];

impl AliasContext {
    fn new(helper_mod: syn::Ident) -> Self {
        Self {
            types: Vec::new(),
            aliases: Vec::new(),
            counter: 0,
            helper_mod,
        }
    }

    /// Produce bridge_type tokens for any Rust type, including references.
    ///
    /// Recursively handles reference and slice types, producing tokens
    /// that resolve via `<T as FfiType>::CRepr` in the cdylib context.
    /// Primitives and `str` are emitted directly; everything else gets
    /// a `$crate::helper_mod::_TypeN` alias for cross-crate resolution.
    fn bridge_tokens(&mut self, ty: &Type) -> proc_macro2::TokenStream {
        match ty {
            Type::Reference(ref_ty) => {
                let inner = self.bridge_tokens(&ref_ty.elem);
                if ref_ty.mutability.is_some() {
                    quote! { &'static mut #inner }
                } else {
                    quote! { &'static #inner }
                }
            }
            Type::Slice(sl) => {
                let elem = self.bridge_tokens(&sl.elem);
                quote! { [#elem] }
            }
            Type::Path(tp) if tp.path.is_ident("str") => quote! { str },
            _ => self.alias_tokens(ty),
        }
    }

    /// Get or create an alias for a non-reference, non-slice, non-keyword type.
    fn alias_tokens(&mut self, ty: &Type) -> proc_macro2::TokenStream {
        if is_primitive(ty) {
            return quote! { #ty };
        }
        let ty_str = quote!(#ty).to_string();
        for (i, existing) in self.types.iter().enumerate() {
            if quote!(#existing).to_string() == ty_str {
                let alias = &self.aliases[i];
                let helper = &self.helper_mod;
                return quote! { $crate::#helper::#alias };
            }
        }
        let alias = format_ident!("_Type{}", self.counter);
        self.counter += 1;
        self.types.push(ty.clone());
        self.aliases.push(alias.clone());
        let helper = &self.helper_mod;
        quote! { $crate::#helper::#alias }
    }

    /// Emit `pub type _TypeN = $crate::Erased;` items for the `@reexport` arm
    /// of the metadata macro. Uses `$crate::` so the module can be recreated
    /// at the crate root by `library_definition!`.
    fn reexport_items_crate(&self) -> Vec<proc_macro2::TokenStream> {
        self.types
            .iter()
            .zip(self.aliases.iter())
            .map(|(ty, alias)| {
                let erased = erase_lifetimes(ty);
                quote! { pub type #alias = $crate::#erased; }
            })
            .collect()
    }
}

fn is_primitive(ty: &Type) -> bool {
    let Type::Path(tp) = ty else { return false };
    tp.path.segments.len() == 1
        && PRIMITIVES.contains(&tp.path.segments[0].ident.to_string().as_str())
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
        ParamKind::ImplTrait {
            trait_name,
            dispatch,
        } => {
            let dispatch_ident = format_ident!("{dispatch}");
            quote! { impl_trait, trait_name = #trait_name, dispatch = #dispatch_ident }
        }
    }
}

fn emit_return_kind(ret: &ReturnKind) -> proc_macro2::TokenStream {
    match ret {
        ReturnKind::Void => quote! { void },
        ReturnKind::Value(bridge_type) => {
            quote! { value(regular, bridge_type = (#bridge_type),) }
        }
        ReturnKind::Result { ok_ty, err_ident } => {
            let ok_tokens = match ok_ty {
                None => quote! { ok = void },
                Some(bridge_type) => {
                    quote! { ok = some(regular, bridge_type = (#bridge_type),) }
                }
            };
            quote! { result(#ok_tokens, err_ident = #err_ident,) }
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
    let mut variant_meta_tokens = Vec::new();

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
        variant_meta_tokens.push(quote! {
            { name = #var_ident, code = #code, message = #message, }
        });
    }

    let unknown_msg = format!(
        "unknown {} error\0",
        camel_to_snake(&name.to_string()).replace('_', " ")
    );
    let unknown_lit = proc_macro2::Literal::byte_string(unknown_msg.as_bytes());

    let error_snake = camel_to_snake(&name.to_string());
    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{error_snake}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{name}");

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

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            (@reexport) => {};
            // Tagged invocation (from library_definition! shim): includes type_tag
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @error,
                    name = #name,
                    path = (#error_path),
                    prefix = $prefix,
                    type_tag = $type_tag,
                    variants = [#(#variant_meta_tokens),*],
                } $(, $($rest)*)? }
            };
            // Untagged invocation (legacy / direct): type_tag defaults to 0
            ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @error,
                    name = #name,
                    path = (#error_path),
                    prefix = $prefix,
                    type_tag = 0,
                    variants = [#(#variant_meta_tokens),*],
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
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

/// Extract the trait name from an `impl Trait` type.
fn extract_impl_trait_name(ty: &Type) -> Option<String> {
    if let Type::ImplTrait(impl_trait) = ty {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(trait_bound) = bound
                && let Some(seg) = trait_bound.path.segments.last()
            {
                return Some(seg.ident.to_string());
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
    supers: Vec<SupertraitBlock>,
}

struct SupertraitBlock {
    trait_name: syn::Ident,
    methods: Vec<syn::TraitItemFn>,
}

impl Parse for ImplementableArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut supers = Vec::new();

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;

            if ident == "prefix" {
                input.parse::<Token![=]>()?;
                let _lit: LitStr = input.parse()?;
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

        Ok(Self { supers })
    }
}

/// A method signature extracted from a trait for vtable generation.
struct VtableMethod {
    name: syn::Ident,
    params: Vec<VtableMethodParam>,
    /// bridge_type for return (None = void)
    ret_bridge_type: Option<proc_macro2::TokenStream>,
    /// rust_type for return (None = void)
    ret_rust_type: Option<proc_macro2::TokenStream>,
    /// Whether this method has a default implementation in the trait.
    has_default: bool,
}

struct VtableMethodParam {
    ident: syn::Ident,
    bridge_type: proc_macro2::TokenStream,
    rust_type: proc_macro2::TokenStream,
}

fn extract_vtable_methods(
    trait_item: &ItemTrait,
    supers: &[SupertraitBlock],
    ctx: &mut AliasContext,
) -> Vec<VtableMethod> {
    let mut methods = Vec::new();

    for item in &trait_item.items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        let has_default = method.default.is_some();
        if let Some(vm) = parse_trait_method_sig(&method.sig, ctx, has_default) {
            methods.push(vm);
        }
    }

    // Supertrait methods are always required (no defaults — the supers(...)
    // syntax only declares signatures, not default bodies).
    for sup in supers {
        for method in &sup.methods {
            if let Some(vm) = parse_trait_method_sig(&method.sig, ctx, false) {
                methods.push(vm);
            }
        }
    }

    methods
}

fn parse_trait_method_sig(sig: &syn::Signature, ctx: &mut AliasContext, has_default: bool) -> Option<VtableMethod> {
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
            let bridge_type = ctx.bridge_tokens(&pt.ty);
            let erased = erase_lifetimes(&pt.ty);
            Some(VtableMethodParam {
                ident: pi.ident.clone(),
                bridge_type,
                rust_type: quote! { #erased },
            })
        })
        .collect();

    let (ret_bridge_type, ret_rust_type) = match &sig.output {
        ReturnType::Default => (None, None),
        ReturnType::Type(_, ty) => {
            let bt = ctx.bridge_tokens(ty);
            let erased = erase_lifetimes(ty);
            (Some(bt), Some(quote! { #erased }))
        }
    };

    Some(VtableMethod {
        name: sig.ident.clone(),
        params,
        ret_bridge_type,
        ret_rust_type,
        has_default,
    })
}

/// Emit metadata tokens for a list of vtable methods (shared by `#[implementable]` and `#[trait_impl]`).
fn emit_vtable_method_meta(methods: &[VtableMethod]) -> Vec<proc_macro2::TokenStream> {
    methods
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
            let hd = m.has_default;
            quote! {
                { name = #mname, params = [#(#param_meta),*], ret = #ret, has_default = #hd, }
            }
        })
        .collect()
}

/// Generate `impl Trait for FfierBoxDyn<dyn Trait>` — delegates each method to `self.0`.
fn emit_boxdyn_impl(trait_item: &ItemTrait) -> proc_macro2::TokenStream {
    let trait_name = &trait_item.ident;
    let method_impls: Vec<_> = trait_item
        .items
        .iter()
        .filter_map(|item| {
            let TraitItem::Fn(method) = item else {
                return None;
            };
            let sig = &method.sig;
            if !matches!(sig.inputs.first(), Some(FnArg::Receiver(_))) {
                return None;
            }
            let name = &sig.ident;
            let params: Vec<_> = sig
                .inputs
                .iter()
                .skip(1)
                .filter_map(|arg| {
                    let FnArg::Typed(pt) = arg else { return None };
                    let Pat::Ident(pi) = &*pt.pat else {
                        return None;
                    };
                    Some(pi.ident.clone())
                })
                .collect();
            Some(quote! { #sig { self.0.#name(#(#params),*) } })
        })
        .collect();

    quote! {
        impl #trait_name for ffier::FfierBoxDyn<dyn #trait_name> {
            #(#method_impls)*
        }
    }
}

/// Generate `impl Trait for FfierBoxDyn<dyn Trait>` for dynamic dispatch fallback.
///
/// `#[ffier::implementable]` implies `#[ffier::dispatch]` — use this
/// annotation alone when you want dynamic dispatch fallback without
/// exporting the trait's vtable to C.
#[proc_macro_attribute]
pub fn dispatch(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let trait_item = parse_macro_input!(item as ItemTrait);
    let original_trait = trait_item.clone();
    let boxdyn_impl = emit_boxdyn_impl(&trait_item);

    let output = quote! {
        #original_trait
        #boxdyn_impl
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
    let tag_const_name = format_ident!("__ffier_type_tag_{trait_name}");
    let tag_const = quote! { crate::#tag_const_name };
    let wrapper_ffi_handle_impls =
        emit_ffi_handle_impls(&quote! { #wrapper_name }, &wrapper_c_handle_suffix, &tag_const);

    let helper_mod_name = format_ident!("_ffier_vtable_{trait_snake}");
    let mut ctx = AliasContext::new(helper_mod_name.clone());

    // Extract all methods (trait + supertraits).
    // own_method_count tracks how many belong to this trait (before supers).
    let vtable_methods = extract_vtable_methods(&trait_item, &args.supers, &mut ctx);
    let own_method_count = trait_item
        .items
        .iter()
        .filter(|item| matches!(item, TraitItem::Fn(_)))
        .count();

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
                pub #name: Option<unsafe extern "C" fn(*mut core::ffi::c_void, #(#params),*) #ret>
            }
        })
        .collect();

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

    // Helper: generate the vtable call expression for a method (unwrapping Option).
    // `fallback` is Some(tokens) for defaulted methods, None for required methods.
    // `method_index` is used for the default_mask bitmask (only relevant for defaulted methods).
    let vtable_call_body = |vm: &VtableMethod,
                            sig: &syn::Signature,
                            fallback: Option<proc_macro2::TokenStream>,
                            method_index: usize|
     -> proc_macro2::TokenStream {
        let name = &vm.name;
        let name_str = name.to_string();
        let vtable_args: Vec<_> = vm
            .params
            .iter()
            .map(|p| {
                let id = &p.ident;
                let bt = &p.bridge_type;
                quote! { <#bt as ffier::FfiType>::into_c(#id) }
            })
            .collect();
        let raw_call = quote! {
            unsafe { __f(self.user_data, #(#vtable_args),*) }
        };
        let vtable_branch = match &vm.ret_bridge_type {
            None => raw_call,
            Some(bt) => quote! { <#bt as ffier::FfiType>::from_c(#raw_call) },
        };
        match fallback {
            Some(fb) => {
                // Defaulted method: use re-entrancy detection + caching.
                //
                // AtomicU64 layout:
                //   low 32 bits  = default_mask  (bit N = method N cached as default)
                //   high 32 bits = in_flight_mask (bit N = method N currently in trampoline)
                //
                // method_index is the index among defaulted methods only.
                let default_bit = 1u64 << method_index;
                let in_flight_bit = 1u64 << (method_index + 32);
                quote! {
                    #sig {
                        use core::sync::atomic::Ordering;
                        let __state = self.vtable_default_state.load(Ordering::Acquire);
                        // Fast path: cached as default
                        if __state & #default_bit != 0 {
                            return #fb;
                        }
                        match unsafe { (*self.vtable).#name } {
                            Some(__f) => {
                                if __state & #in_flight_bit != 0 {
                                    // Re-entering — client doesn't override this method.
                                    // Cache as default for future calls.
                                    self.vtable_default_state.fetch_or(#default_bit, Ordering::Release);
                                    #fb
                                } else {
                                    // Set in-flight bit before calling trampoline
                                    self.vtable_default_state.fetch_or(#in_flight_bit, Ordering::Release);
                                    let __call_result = { #vtable_branch };
                                    // Clear in-flight bit
                                    self.vtable_default_state.fetch_and(!#in_flight_bit, Ordering::Release);
                                    __call_result
                                }
                            }
                            None => {
                                self.vtable_default_state.fetch_or(#default_bit, Ordering::Release);
                                #fb
                            }
                        }
                    }
                }
            }
            None => {
                // Required method: no re-entrancy detection needed.
                let wrapper_str = wrapper_name.to_string();
                quote! {
                    #sig {
                        match unsafe { (*self.vtable).#name } {
                            Some(__f) => { #vtable_branch }
                            None => {
                                panic!(
                                    "{}: required vtable method `{}` not provided",
                                    #wrapper_str, #name_str,
                                )
                            }
                        }
                    }
                }
            }
        }
    };

    // --- Default method extraction ---
    // For methods with default bodies, extract the body into a free helper function
    // and rewrite the trait's default to call it. This allows the VtableXxx impl's
    // None branch to also call the helper, preserving the default behavior when
    // the C side doesn't provide a function pointer.
    //
    // Visitor to replace `self` with `__self` in default method bodies.
    struct SelfReplacer;
    impl VisitMut for SelfReplacer {
        fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
            if let syn::Expr::Path(ep) = expr {
                if ep.qself.is_none() && ep.path.is_ident("self") {
                    *expr = syn::parse_quote! { __self };
                    return;
                }
            }
            syn::visit_mut::visit_expr_mut(self, expr);
        }
    }

    let mut default_helpers: Vec<proc_macro2::TokenStream> = Vec::new();
    // Map from method name → helper fn ident (only for methods with defaults)
    let mut default_helper_names: HashMap<String, syn::Ident> = HashMap::new();

    // Build modified trait: replace default bodies with helper calls
    let mut modified_trait = original_trait.clone();
    for item in &mut modified_trait.items {
        let TraitItem::Fn(method) = item else { continue };
        let Some(default_block) = &method.default else { continue };

        let method_name = &method.sig.ident;
        let helper_name = format_ident!("__ffier_default_{trait_name}_{method_name}");
        default_helper_names.insert(method_name.to_string(), helper_name.clone());

        // Extract the default body, rewriting self → __self
        let mut body = default_block.clone();
        SelfReplacer.visit_block_mut(&mut body);

        // Build the helper function signature: same as the trait method but
        // with &self replaced by __self: &(impl Trait + ?Sized)
        let mut helper_sig = method.sig.clone();
        // Remove the receiver and add __self parameter
        let helper_params: Vec<_> = helper_sig
            .inputs
            .iter()
            .filter(|arg| !matches!(arg, syn::FnArg::Receiver(_)))
            .cloned()
            .collect();
        helper_sig.inputs = syn::punctuated::Punctuated::new();
        helper_sig.inputs.push(syn::parse_quote! {
            __self: &(impl #trait_name + ?Sized)
        });
        for p in helper_params {
            helper_sig.inputs.push(p);
        }
        helper_sig.ident = helper_name.clone();

        default_helpers.push(quote! {
            #[doc(hidden)]
            #helper_sig #body
        });

        // Rewrite the trait's default body to call the helper
        let params_pass: Vec<_> = method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| {
                if let syn::FnArg::Typed(pat_type) = arg {
                    if let syn::Pat::Ident(pi) = &*pat_type.pat {
                        return Some(pi.ident.clone());
                    }
                }
                None
            })
            .collect();
        method.default = Some(syn::parse_quote! {
            { #helper_name(self #(, #params_pass)*) }
        });
    }

    // --- Generate VtableXxx method impls ---
    let own_method_impls: Vec<_> = trait_item_erased
        .items
        .iter()
        .filter_map(|item| {
            let TraitItem::Fn(method) = item else {
                return None;
            };
            let name = &method.sig.ident;
            let method_index = vtable_methods.iter().position(|v| v.name == *name)?;
            let vm = &vtable_methods[method_index];
            let fallback = default_helper_names.get(&name.to_string()).map(|helper| {
                let params_pass: Vec<_> = method
                    .sig
                    .inputs
                    .iter()
                    .filter_map(|arg| {
                        if let syn::FnArg::Typed(pat_type) = arg {
                            if let syn::Pat::Ident(pi) = &*pat_type.pat {
                                return Some(pi.ident.clone());
                            }
                        }
                        None
                    })
                    .collect();
                quote! { #helper(self #(, #params_pass)*) }
            });
            Some(vtable_call_body(vm, &method.sig, fallback, method_index))
        })
        .collect();

    // Supertrait impls — all required (supers don't have defaults)
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
                    let method_index = vtable_methods.iter().position(|v| v.name == *name)?;
                    let vm = &vtable_methods[method_index];
                    Some(vtable_call_body(vm, &method.sig, None, method_index))
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
    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{trait_snake}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{trait_name}");

    let vtable_method_meta = emit_vtable_method_meta(&vtable_methods);

    let trait_path_tokens = quote! { $crate::#trait_name };

    let reexport_items_crate = ctx.reexport_items_crate();

    // Generate FfierBoxDyn delegation (implies #[ffier::dispatch])
    // For traits with supertraits, we also need to delegate the supertrait methods.
    let has_supertraits = !args.supers.is_empty()
        || trait_item
            .supertraits
            .iter()
            .any(|b| matches!(b, syn::TypeParamBound::Trait(_)));

    let boxdyn_impl = if !has_supertraits {
        emit_boxdyn_impl(&trait_item)
    } else {
        // TODO: generate supertrait delegation for FfierBoxDyn
        quote! {}
    };

    let output = quote! {
        #(#default_helpers)*

        #modified_trait

        #boxdyn_impl

        #[repr(C)]
        pub struct #vtable_struct_name {
            /// Drop callback. Always the first field so its offset is stable
            /// across vtable versions (forward-compatible ABI).
            pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
            #(#vtable_fields,)*
        }

        pub struct #wrapper_name {
            pub user_data: *mut core::ffi::c_void,
            pub vtable: *const #vtable_struct_name,
            /// Packed bitmask for vtable default method detection (AtomicU64):
            /// - Low 32 bits: default_mask — bit N set = defaulted method N
            ///   is cached as "uses library default" (skip trampoline).
            /// - High 32 bits: in_flight_mask — bit N set = defaulted method N
            ///   is currently being called (re-entrancy guard).
            ///
            /// Only defaulted methods get bits (not required methods).
            /// Limits: 32 defaulted methods per trait.
            #[doc(hidden)]
            pub vtable_default_state: core::sync::atomic::AtomicU64,
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

        #wrapper_ffi_handle_impls

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            (@reexport) => {
                #[doc(hidden)]
                pub mod #helper_mod_name {
                    #(#reexport_items_crate)*
                }
            };
            // Tagged invocation (from library_definition! shim): includes type_tag
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @implementable,
                    trait_name = #trait_name,
                    trait_path = (#trait_path_tokens),
                    prefix = $prefix,
                    type_tag = $type_tag,
                    vtable_struct = ($crate::#vtable_struct_name),
                    wrapper_name = ($crate::#wrapper_name),
                    vtable_methods = [#(#vtable_method_meta),*],
                    own_method_count = #own_method_count,
                } $(, $($rest)*)? }
            };
            // Untagged invocation (legacy / direct): type_tag defaults to 0
            ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @implementable,
                    trait_name = #trait_name,
                    trait_path = (#trait_path_tokens),
                    prefix = $prefix,
                    type_tag = 0,
                    vtable_struct = ($crate::#vtable_struct_name),
                    wrapper_name = ($crate::#wrapper_name),
                    vtable_methods = [#(#vtable_method_meta),*],
                    own_method_count = #own_method_count,
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
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
    let struct_last_seg = struct_type_path
        .path
        .segments
        .last()
        .expect("expected struct name");
    let struct_ident = &struct_last_seg.ident;
    let struct_name = struct_ident.to_string();
    let struct_snake = camel_to_snake(&struct_name);

    // Extract lifetime arguments from the struct type (e.g. 'a from View<'a>).
    // Structs without lifetime params (e.g. Widget) will produce an empty list.
    let struct_lt_args: Vec<String> = match &struct_last_seg.arguments {
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

    let helper_mod_name = format_ident!("_ffier_impl_{trait_snake}_for_{struct_snake}");
    let mut ctx = AliasContext::new(helper_mod_name.clone());

    // Extract methods, skipping any marked with #[ffier(skip)].
    let methods: Vec<VtableMethod> = input
        .items
        .iter()
        .filter_map(|item| {
            let ImplItem::Fn(method) = item else {
                return None;
            };
            if method.attrs.iter().any(is_ffier_skip) {
                return None;
            }
            // trait_impl methods are concrete overrides, not defaults
            parse_trait_method_sig(&method.sig, &mut ctx, false)
        })
        .collect();

    let reexport_items_crate = ctx.reexport_items_crate();

    let method_meta = emit_vtable_method_meta(&methods);

    let lifetime_idents: Vec<_> = input
        .generics
        .lifetimes()
        .map(|lt| format_ident!("{}", lt.lifetime.ident))
        .collect();

    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name =
        format_ident!("__ffier_internal_{trait_snake}_for_{struct_snake}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{trait_name}_for_{struct_ident}");
    let struct_path_tokens = quote! { $crate::#struct_ident };
    let trait_path_tokens = quote! { $crate::#trait_name };

    let output = quote! {
        #clean_impl

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            (@reexport) => {
                #[doc(hidden)]
                pub mod #helper_mod_name {
                    #(#reexport_items_crate)*
                }
            };
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
                    struct_lifetime_args = [#(#struct_lt_args),*],
                    methods = [#(#method_meta),*],
                } $(, $($rest)*)? }
            };
        }

        #[doc(hidden)]
        pub use #internal_macro_name as #meta_alias_name;
    };

    output.into()
}

// ===========================================================================
// ffier::library_definition! — define a library's exported types
// ===========================================================================

/// Define the list of exported types for a library.
///
/// Every entry (except `TraitName for StructName`) must have an explicit
/// type tag: `Name = N`. Tags must be nonzero and unique across the library.
/// Entries can be bare paths or qualified paths (e.g. `crate::submod::Foo`).
///
/// ```ignore
/// ffier::library_definition!("mylib",
///     CalcError = 1,
///     Calculator = 2,
///     crate::submod::TextBuffer = 3,
///     BufferError = 4,
///     trait Processor = 10,
///     trait crate::traits::Fruit = 11,
///     crate::traits::Fruit for crate::types::Apple,
/// );
///
/// // In cdylib:
/// mylib::__ffier_mylib_library!(ffier_gen_c_macros::generate);
/// ```
///
/// Supports three entry kinds:
/// - `Path = N` — exportable struct or error enum with type tag
/// - `trait Path = N` — implementable trait with type tag
/// - `TraitPath for StructPath` — trait impl bridge (uses the struct's tag)
///
/// Each annotated type generates a `__ffier_meta_*` alias macro next to the
/// type via `pub use`. This macro resolves those aliases from the given paths
/// and invokes their `@reexport` arm to create helper modules at the crate root.
#[proc_macro]
pub fn library_definition(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as LibraryInput);
    let prefix_lit = &parsed.prefix;
    let prefix_str = parsed.prefix.value();

    // For each entry, compute:
    // 1. tag_consts: `pub const __ffier_type_tag_Foo: u32 = N;` emissions
    // 2. shim_macros: shim macros that inject the tag into the metadata macro call
    // 3. reexport_invocations: `alias_path!(@reexport)` calls
    // 4. chain_paths: the macro path used inside the generated library macro
    let mut tag_consts: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut shim_macros: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut reexport_invocations: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut chain_paths: Vec<proc_macro2::TokenStream> = Vec::new();

    for entry in &parsed.entries {
        match entry {
            LibraryEntry::Tagged(path, tag) => {
                let last_ident = path_last_ident(path);
                let tag_const_ident = format_ident!("__ffier_type_tag_{last_ident}");
                tag_consts.push(quote! {
                    #[doc(hidden)]
                    pub const #tag_const_ident: u32 = #tag;
                });

                // Shim macro that injects the tag into the metadata macro call
                let alias = meta_alias_for_type(path);
                let alias_chain = to_chain_path(&alias);
                let shim_name = format_ident!("__ffier_tagged_{prefix_str}_{last_ident}");
                shim_macros.push(quote! {
                    #[doc(hidden)]
                    #[macro_export]
                    macro_rules! #shim_name {
                        ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                            #alias_chain! { $prefix, #tag, $callback $(, $($rest)*)? }
                        };
                    }
                });

                // @reexport invocation uses the alias path at the library crate root
                reexport_invocations.push(quote! { #alias!(@reexport); });
                // chain_path is the shim (which injects the tag)
                chain_paths.push(quote! { $crate::#shim_name });
            }
            LibraryEntry::TaggedTrait(path, tag) => {
                let last_ident = path_last_ident(path);
                let tag_const_ident = format_ident!("__ffier_type_tag_{last_ident}");
                tag_consts.push(quote! {
                    #[doc(hidden)]
                    pub const #tag_const_ident: u32 = #tag;
                });

                // Shim macro for trait entries
                let alias = meta_alias_for_type(path);
                let alias_chain = to_chain_path(&alias);
                let shim_name = format_ident!("__ffier_tagged_{prefix_str}_{last_ident}");
                shim_macros.push(quote! {
                    #[doc(hidden)]
                    #[macro_export]
                    macro_rules! #shim_name {
                        ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                            #alias_chain! { $prefix, #tag, $callback $(, $($rest)*)? }
                        };
                    }
                });

                reexport_invocations.push(quote! { #alias!(@reexport); });
                chain_paths.push(quote! { $crate::#shim_name });
            }
            LibraryEntry::TraitImpl {
                trait_path,
                struct_path,
            } => {
                let trait_name = path_last_ident(trait_path);
                let struct_name = path_last_ident(struct_path);
                let alias_ident = format_ident!("__ffier_meta_{trait_name}_for_{struct_name}");
                // The trait_impl alias lives next to the struct (where the impl block is)
                let alias = replace_last_segment(struct_path, &alias_ident);
                let chain = to_chain_path(&alias);
                reexport_invocations.push(quote! { #alias!(@reexport); });
                chain_paths.push(chain);
            }
        }
    }

    if chain_paths.is_empty() {
        return quote! { compile_error!("library_definition! requires at least one type"); }.into();
    }

    let first = &chain_paths[0];
    let rest = &chain_paths[1..];

    let entry_macro_name = format_ident!("__ffier_{prefix_str}_library");

    let output = quote! {
        #(#tag_consts)*

        // Create _ffier_* helper modules at the crate root via @reexport.
        #(#reexport_invocations)*

        #(#shim_macros)*

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
                #first! { #prefix_lit, $crate::__ffier_chain,
                    #prefix_lit, $callback,
                    [],
                    [#(#rest),*]
                }
            };
        }
    };

    output.into()
}

// ---------------------------------------------------------------------------
// library_definition! helpers
// ---------------------------------------------------------------------------

/// Build the `__ffier_meta_*` alias path for a plain type or trait path.
/// Replaces the last segment: `a::b::Foo` → `a::b::__ffier_meta_Foo`.
fn meta_alias_for_type(path: &syn::Path) -> syn::Path {
    let name = path_last_ident(path);
    let alias_ident = format_ident!("__ffier_meta_{name}");
    replace_last_segment(path, &alias_ident)
}

/// Convert an alias path to a chain-usable path.
///
/// - Bare ident `__ffier_meta_Foo` → `$crate::__ffier_meta_Foo`
/// - `crate::a::b::__ffier_meta_Foo` → `$crate::a::b::__ffier_meta_Foo`
/// - `other_crate::__ffier_meta_Foo` → `other_crate::__ffier_meta_Foo` (as-is)
fn to_chain_path(path: &syn::Path) -> proc_macro2::TokenStream {
    let first_seg = &path.segments.first().unwrap().ident;

    if path.segments.len() == 1 {
        // Bare ident — same crate
        quote! { $crate::#path }
    } else if first_seg == "crate" {
        // crate::a::b::X → $crate::a::b::X
        let without_crate: syn::punctuated::Punctuated<syn::PathSegment, Token![::]> =
            path.segments.iter().skip(1).cloned().collect();
        quote! { $crate::#without_crate }
    } else {
        // External crate path — use as-is
        quote! { #path }
    }
}

/// Extract the last identifier from a path.
fn path_last_ident(path: &syn::Path) -> &syn::Ident {
    &path.segments.last().expect("path must have segments").ident
}

/// Replace the last segment of a path with a new ident.
fn replace_last_segment(path: &syn::Path, new_last: &syn::Ident) -> syn::Path {
    let mut result = path.clone();
    let last = result.segments.last_mut().unwrap();
    last.ident = new_last.clone();
    last.arguments = syn::PathArguments::None;
    result
}

// ---------------------------------------------------------------------------
// library_definition! parsing
// ---------------------------------------------------------------------------

struct LibraryInput {
    prefix: LitStr,
    entries: Vec<LibraryEntry>,
}

enum LibraryEntry {
    /// A plain type or error with an explicit type tag: `Path = N`
    Tagged(syn::Path, u32),
    /// An implementable trait with an explicit type tag: `trait Path = N`
    TaggedTrait(syn::Path, u32),
    /// A trait impl bridge: `TraitPath for StructPath`
    TraitImpl {
        trait_path: syn::Path,
        struct_path: syn::Path,
    },
}

/// Parse a `= N` type tag after an identifier. Returns the tag value.
/// Rejects tag 0 (reserved for success / no-type).
fn parse_type_tag(input: syn::parse::ParseStream) -> syn::Result<u32> {
    input.parse::<Token![=]>()?;
    let tag_lit: syn::LitInt = input.parse()?;
    let tag = tag_lit.base10_parse::<u32>()?;
    if tag == 0 {
        return Err(syn::Error::new(
            tag_lit.span(),
            "type tag must be nonzero (0 is reserved)",
        ));
    }
    Ok(tag)
}

impl Parse for LibraryInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let prefix: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;

        let mut entries = Vec::new();
        while !input.is_empty() {
            if input.peek(Token![trait]) {
                // `trait Path = N`
                input.parse::<Token![trait]>()?;
                let path: syn::Path = input.parse()?;
                let tag = parse_type_tag(input)?;
                entries.push(LibraryEntry::TaggedTrait(path, tag));
            } else {
                let first: syn::Path = input.parse()?;
                if input.peek(Token![for]) {
                    // `TraitPath for StructPath`
                    input.parse::<Token![for]>()?;
                    let second: syn::Path = input.parse()?;
                    entries.push(LibraryEntry::TraitImpl {
                        trait_path: first,
                        struct_path: second,
                    });
                } else {
                    // `Path = N`
                    let tag = parse_type_tag(input)?;
                    entries.push(LibraryEntry::Tagged(first, tag));
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(LibraryInput { prefix, entries })
    }
}

// ===========================================================================
// #[ffier::reexport] — re-export ffier metadata alongside types
// ===========================================================================

/// Attribute for `pub use` statements that re-export ffier-annotated types.
///
/// When you re-export a type from another crate (or from a private submodule),
/// the ffier metadata macro alias needs to travel with it. This attribute
/// automatically generates the corresponding `pub use` for the metadata.
///
/// ```ignore
/// #[ffier::reexport]
/// pub use other_crate::Calculator;
/// // also generates: pub use other_crate::__ffier_meta_Calculator;
///
/// #[ffier::reexport]
/// pub use other_crate::{Calculator, OtherThing};
/// // generates both metadata re-exports
/// ```
#[proc_macro_attribute]
pub fn reexport(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let use_item = parse_macro_input!(item as ItemUse);

    let mut meta_reexports = Vec::new();
    collect_reexport_paths(&use_item.tree, &mut Vec::new(), &mut meta_reexports);

    let output = quote! {
        #use_item

        #(
            #[doc(hidden)]
            #meta_reexports
        )*
    };

    output.into()
}

/// Walk a `UseTree` and collect `pub use path::__ffier_meta_Type;` statements
/// for each simple (non-glob, non-renamed) leaf.
fn collect_reexport_paths(
    tree: &syn::UseTree,
    prefix: &mut Vec<syn::Ident>,
    out: &mut Vec<proc_macro2::TokenStream>,
) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.clone());
            collect_reexport_paths(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let type_name = &n.ident;
            let meta_alias = format_ident!("__ffier_meta_{type_name}");
            if prefix.is_empty() {
                out.push(quote! { pub use #meta_alias; });
            } else {
                out.push(quote! { pub use #(#prefix)::*::#meta_alias; });
            }
        }
        syn::UseTree::Group(g) => {
            for tree in &g.items {
                collect_reexport_paths(tree, prefix, out);
            }
        }
        syn::UseTree::Rename(_) => {
            // Renamed imports are not supported — the user can manually
            // re-export the metadata if needed.
        }
        syn::UseTree::Glob(_) => {
            // Glob imports can't be handled — we don't know which types exist.
        }
    }
}
