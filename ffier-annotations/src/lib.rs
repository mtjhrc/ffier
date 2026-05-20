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

// ---------------------------------------------------------------------------
// Unified method/param/return types — shared by all annotation macros
// ---------------------------------------------------------------------------

/// A bridge/rust type pair. Used for params and return types.
struct TypePair {
    bridge: proc_macro2::TokenStream,
    rust: proc_macro2::TokenStream,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Receiver {
    /// Static method (no receiver).
    None,
    /// `&self`
    Ref,
    /// `&mut self`
    Mut,
    /// `self` (consuming, by value)
    Value,
}

enum ParamKind {
    /// Uniform: bridge_type resolves via `<T as FfiType>::CRepr`.
    Regular,
    /// `&[&str]` — slice of string references, expands to two C params.
    StrSlice,
    /// `impl Trait` parameter — generator resolves dispatch types from trait map.
    ImplTrait {
        trait_name: String,
        /// Dispatch mode: "auto", "concrete", or "vtable".
        /// For trait methods this defaults to "auto".
        dispatch: String,
        /// Lifetime arguments on the trait (e.g. `["a"]` for `impl Snapshot<'a>`).
        trait_lifetime_args: Vec<String>,
    },
}

enum ReturnKind {
    Void,
    Value(TypePair),
    Result {
        ok: Option<TypePair>,
        err_ident: String,
    },
}

struct ParamInfo {
    name: syn::Ident,
    kind: ParamKind,
    /// Type pair for this param. Present for `Regular` and `ImplTrait`,
    /// `None` for `StrSlice` (which expands to two C params).
    types: Option<TypePair>,
}

struct MethodInfo {
    name: syn::Ident,
    receiver: Receiver,
    params: Vec<ParamInfo>,
    ret: ReturnKind,
    // --- exportable-specific (defaults for trait methods) ---
    /// FFI function name suffix (e.g. `"widget_new"`). Empty for trait methods.
    ffi_name: String,
    /// True if this method returns Self (builder pattern).
    is_builder: bool,
    /// Method-level lifetime params.
    method_lifetimes: Vec<syn::Ident>,
    doc_lines: Vec<String>,
    /// Original Rust return type for client codegen.
    rust_ret: Option<proc_macro2::TokenStream>,
    // --- trait-specific (defaults for exportable methods) ---
    /// Whether this method has a default impl in the trait.
    has_default: bool,
    /// Vtable slot index.
    index: usize,
    /// Raw handle method (receives `*const FfierHandle<Self>` instead of `&self`).
    raw_handle: bool,
}

impl ParamInfo {
    fn is_impl_trait(&self) -> bool {
        matches!(self.kind, ParamKind::ImplTrait { .. })
    }

    /// Bridge type tokens. Panics for `StrSlice` (which has no single bridge type).
    fn bridge_type(&self) -> &proc_macro2::TokenStream {
        &self.types.as_ref().expect("StrSlice has no bridge_type").bridge
    }

    /// Rust type tokens. Panics for `StrSlice`.
    fn rust_type(&self) -> &proc_macro2::TokenStream {
        &self.types.as_ref().expect("StrSlice has no rust_type").rust
    }
}

impl MethodInfo {
    /// Bridge type for the return value. None for void returns.
    fn ret_bridge_type(&self) -> Option<&proc_macro2::TokenStream> {
        match &self.ret {
            ReturnKind::Void => None,
            ReturnKind::Value(tp) => Some(&tp.bridge),
            ReturnKind::Result { ok, .. } => ok.as_ref().map(|tp| &tp.bridge),
        }
    }

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

/// Emit `impl FfiHandle` and `impl FfiType` for a handle type.
///
/// `ty` is the type (e.g. `Widget` or `VtableFruit`), `c_handle_name` is the
/// C handle typedef name string (e.g. `"Widget"`, `"VtableFruit"`).
///
/// `tag_const` is a token stream referencing the type tag constant, e.g.
/// `crate::__ffier_type_tag_Widget`. The actual value is provided by
/// `library_definition!` which emits `pub const __ffier_type_tag_Widget: u32 = N;`.
///
/// All handles are heap-allocated via `Box<FfierHandle<T>>`. `as_handle`
/// recovers the handle pointer by subtracting the value field offset from
/// `&self`. `into_c` allocates a new handle box and returns the raw pointer.
fn emit_ffi_handle_impls(
    ty: &proc_macro2::TokenStream,
    c_handle_name: &str,
    tag_const: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    quote! {
        impl ffier::FfiHandle for #ty {
            const C_HANDLE_NAME: &str = #c_handle_name;
            const TYPE_TAG: u32 = #tag_const;
            unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
                let ptr = (self as *const Self as *const u8)
                    .wrapping_sub(ffier::HANDLE_VALUE_OFFSET);
                ptr as *mut core::ffi::c_void
            }
        }

        impl ffier::FfiType for #ty {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &str = #c_handle_name;
            const IS_HANDLE: bool = true;
            fn into_c(self) -> *mut core::ffi::c_void {
                ffier::ffier_handle_new(
                    <Self as ffier::FfiHandle>::TYPE_TAG,
                    self,
                )
            }
            fn from_c(repr: *mut core::ffi::c_void) -> Self {
                unsafe { ffier::ffier_handle_consume::<Self>(repr) }
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

        if let Some(mut m) = parse_method_sig(
            &method.sig,
            &method.attrs,
            &mut ctx,
            Some(self_ty),
            false,
            false,
        ) {
            m.ffi_name = format!("{}_{}", struct_lower, method.sig.ident);
            methods.push(m);
        }
    }

    // -----------------------------------------------------------------------
    // Metadata emission — structured tokens for generator proc macros
    // -----------------------------------------------------------------------

    let reexport_items_crate = ctx.reexport_items_crate();

    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{struct_lower}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{struct_ident}");

    let method_meta_tokens = emit_method_meta(&methods);

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
    let ffi_handle_impls =
        emit_ffi_handle_impls(&quote! { #self_ty_static }, &struct_name_lit, &tag_const);

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
        // Static message for strerror: #[ffier(message="...")] > raw variant name
        // Data-carrying variants get "Name(...)" / "Name{..}" to signal
        // that ft_error_message() has richer detail.
        let message = attrs.message.unwrap_or_else(|| {
            let name = var_ident.to_string();
            match &variant.fields {
                syn::Fields::Unit => name,
                syn::Fields::Unnamed(_) => format!("{name}(...)"),
                syn::Fields::Named(_) => format!("{name}{{..}}"),
            }
        });
        let upper_name = camel_to_upper_snake(&var_ident.to_string());

        // Wildcard pattern for variants with fields
        let match_pattern = match &variant.fields {
            syn::Fields::Unit => quote! { #name::#var_ident },
            syn::Fields::Unnamed(_) => quote! { #name::#var_ident(..) },
            syn::Fields::Named(_) => quote! { #name::#var_ident { .. } },
        };
        code_arms.push(quote! { #match_pattern => #code });

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

    // Error types get the same FfiHandle + FfiType impls as exportable types.
    // The type tag constant is emitted by library_definition!.
    let name_str = name.to_string();
    let tag_const_name = format_ident!("__ffier_type_tag_{name}");
    let tag_const = quote! { crate::#tag_const_name };
    let ffi_handle_impls =
        emit_ffi_handle_impls(&quote! { #name }, &name_str, &tag_const);

    let output = quote! {
        #ffi_handle_impls

        impl ffier::FfiError for #name {
            fn code(&self) -> u32 {
                match self {
                    #(#code_arms,)*
                }
            }

            fn static_message(code: u32) -> &'static core::ffi::CStr {
                match code {
                    #(#message_arms,)*
                    _ => unsafe {
                        core::ffi::CStr::from_bytes_with_nul_unchecked(#unknown_lit)
                    },
                }
            }

            fn codes() -> &'static [(&'static str, u32)] {
                &[#(#codes_entries),*]
            }
        }

        #[ffier::trait_impl]
        impl ffier::Error for #name {
            fn code(&self) -> u32 {
                ffier::FfiError::code(self)
            }
            fn message(&self, writer: &mut impl ffier::PushStr) {
                use core::fmt::Write;
                let writer: &mut dyn ffier::PushStr = writer;
                let _ = write!(writer, "{}", self);
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
    code: u32,
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
                code = Some(lit.base10_parse::<u32>()?);
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

/// Parse `#[ffier(dispatch = concrete|vtable)]` from a parameter's attributes.
/// Only `dispatch` is recognized; unknown keys are rejected.
fn parse_ffier_param_dispatch(attrs: &[syn::Attribute]) -> Option<String> {
    let mut result = None;
    for attr in attrs {
        if !attr.path().is_ident("ffier") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("dispatch") {
                let value = meta.value()?;
                let mode: syn::Ident = value.parse()?;
                result = Some(mode.to_string());
            } else {
                return Err(meta.error(format!(
                    "unknown #[ffier] key `{}` on parameter",
                    meta.path.to_token_stream(),
                )));
            }
            Ok(())
        });
    }
    result
}

/// All recognized keys from `#[ffier(...)]` on a trait/impl method.
struct FfierMethodAttrs {
    index: Option<usize>,
    raw_handle: bool,
    dispatch: Option<String>,
    skip: bool,
}

/// Parse all `#[ffier(...)]` attributes on a method in one pass.
/// Rejects unknown keys.
fn parse_ffier_method_attrs(attrs: &[syn::Attribute]) -> syn::Result<FfierMethodAttrs> {
    let mut result = FfierMethodAttrs {
        index: None,
        raw_handle: false,
        dispatch: None,
        skip: false,
    };

    for attr in attrs {
        if !attr.path().is_ident("ffier") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("index") {
                let value = meta.value()?;
                let lit: syn::LitInt = value.parse()?;
                result.index = Some(lit.base10_parse::<usize>()?);
            } else if meta.path.is_ident("raw_handle") {
                result.raw_handle = true;
            } else if meta.path.is_ident("dispatch") {
                let value = meta.value()?;
                let mode: syn::Ident = value.parse()?;
                result.dispatch = Some(mode.to_string());
            } else if meta.path.is_ident("skip") {
                result.skip = true;
            } else {
                return Err(meta.error(format!(
                    "unknown #[ffier] key `{}`",
                    meta.path.to_token_stream(),
                )));
            }
            Ok(())
        })?;
    }

    Ok(result)
}

/// Extract the trait name from an `impl Trait` type.
/// Extract trait name and lifetime arguments from `impl Trait<'a, 'b>`.
/// Returns `(trait_name, lifetime_args)` e.g. `("Snapshot", vec!["a"])`.
fn extract_impl_trait_info(ty: &Type) -> Option<(String, Vec<String>)> {
    if let Type::ImplTrait(impl_trait) = ty {
        for bound in &impl_trait.bounds {
            if let syn::TypeParamBound::Trait(trait_bound) = bound
                && let Some(seg) = trait_bound.path.segments.last()
            {
                let name = seg.ident.to_string();
                let mut lt_args = Vec::new();
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Lifetime(lt) = arg {
                            lt_args.push(lt.ident.to_string());
                        }
                    }
                }
                return Some((name, lt_args));
            }
        }
    }
    None
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
    /// Reserved vtable slot indices (retired methods). These slots are padded
    /// in the vtable struct to keep the layout stable.
    reserved: Vec<usize>,
    /// If true, the trait is foreign (defined in another crate). The macro
    /// will not emit the trait definition or `FfierBoxDyn` impl.
    foreign: bool,
}

struct SupertraitBlock {
    trait_name: syn::Ident,
    methods: Vec<syn::TraitItemFn>,
}

impl Parse for ImplementableArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut supers = Vec::new();
        let mut reserved = Vec::new();
        let mut foreign = false;

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
            } else if ident == "reserved" {
                let content;
                syn::parenthesized!(content in input);
                while !content.is_empty() {
                    let lit: syn::LitInt = content.parse()?;
                    reserved.push(lit.base10_parse::<usize>()?);
                    let _ = content.parse::<Token![,]>();
                }
            } else if ident == "foreign" {
                foreign = true;
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    "expected `prefix`, `supers`, `reserved`, or `foreign`",
                ));
            }
            let _ = input.parse::<Token![,]>();
        }

        Ok(Self { supers, reserved, foreign })
    }
}

// ---------------------------------------------------------------------------
// Unified method parsing — shared by #[exportable], #[implementable], #[trait_impl]
// ---------------------------------------------------------------------------

fn extract_vtable_methods(
    trait_item: &ItemTrait,
    supers: &[SupertraitBlock],
    reserved: &[usize],
    ctx: &mut AliasContext,
) -> syn::Result<Vec<MethodInfo>> {
    let mut methods = Vec::new();

    for item in &trait_item.items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        let has_default = method.default.is_some();
        let mattrs = parse_ffier_method_attrs(&method.attrs)?;
        if let Some(mut m) = parse_method_sig(&method.sig, &method.attrs, ctx, None, has_default, mattrs.raw_handle) {
            let index = mattrs.index.ok_or_else(|| {
                syn::Error::new_spanned(
                    &method.sig.ident,
                    format!(
                        "vtable method `{}` is missing `#[ffier(index = N)]`",
                        method.sig.ident,
                    ),
                )
            })?;
            m.index = index;
            methods.push(m);
        }
    }

    // Supertrait methods are always required (no defaults — the supers(...)
    // syntax only declares signatures, not default bodies).
    for sup in supers {
        for method in &sup.methods {
            let mattrs = parse_ffier_method_attrs(&method.attrs)?;
            if let Some(mut m) = parse_method_sig(&method.sig, &method.attrs, ctx, None, false, mattrs.raw_handle) {
                let index = mattrs.index.ok_or_else(|| {
                    syn::Error::new_spanned(
                        &method.sig.ident,
                        format!(
                            "supertrait vtable method `{}` is missing `#[ffier(index = N)]`",
                            method.sig.ident,
                        ),
                    )
                })?;
                m.index = index;
                methods.push(m);
            }
        }
    }

    // Validate: no duplicate indices
    let mut seen: HashMap<usize, &syn::Ident> = HashMap::new();
    for m in &methods {
        if let Some(prev) = seen.insert(m.index, &m.name) {
            return Err(syn::Error::new_spanned(
                &m.name,
                format!(
                    "duplicate vtable index {}: both `{}` and `{}` use index {}",
                    m.index, prev, m.name, m.index,
                ),
            ));
        }
    }

    // Validate: no method uses a reserved index
    for m in &methods {
        if reserved.contains(&m.index) {
            return Err(syn::Error::new_spanned(
                &m.name,
                format!(
                    "vtable index {} is reserved (retired slot) but used by method `{}`",
                    m.index, m.name,
                ),
            ));
        }
    }

    Ok(methods)
}

/// Parse a single method signature into a unified `MethodInfo`.
///
/// - `attrs`: method-level attributes (for doc comments and `#[ffier(dispatch = ...)]` on params)
/// - `self_ty`: if `Some`, `Self` in param/return types is replaced with this type
///   and builder pattern is detected. Typically `Some` for `#[exportable]`, `None`
///   for `#[implementable]`/`#[trait_impl]`.
/// - `has_default`: whether this method has a default impl body (trait methods only)
/// - `raw_handle`: whether this is a raw-handle method
fn parse_method_sig(
    sig: &syn::Signature,
    attrs: &[syn::Attribute],
    ctx: &mut AliasContext,
    self_ty: Option<&Type>,
    has_default: bool,
    raw_handle: bool,
) -> Option<MethodInfo> {
    // --- Determine receiver ---
    let receiver = if raw_handle {
        Receiver::None
    } else {
        match sig.inputs.first() {
            Some(FnArg::Receiver(r)) => {
                if r.reference.is_none() {
                    Receiver::Value
                } else if r.mutability.is_some() {
                    Receiver::Mut
                } else {
                    Receiver::Ref
                }
            }
            _ if self_ty.is_some() => Receiver::None, // static method in exportable
            _ => return None, // trait method without receiver — skip
        }
    };

    // Skip receiver or raw_handle's first param (the handle pointer)
    let skip_n = if receiver != Receiver::None || raw_handle { 1 } else { 0 };

    // --- Parse params ---
    let params: Vec<_> = sig
        .inputs
        .iter()
        .skip(skip_n)
        .filter_map(|arg| {
            let FnArg::Typed(pt) = arg else { return None };
            let Pat::Ident(pi) = &*pt.pat else {
                return None;
            };

            // Unwrap reference for impl Trait detection:
            // `&mut impl PushStr` → inner type is `impl PushStr`
            let inner_ty = match &*pt.ty {
                Type::Reference(r) => &*r.elem,
                other => other,
            };

            if let Some((trait_name, trait_lifetime_args)) = extract_impl_trait_info(inner_ty) {
                let dispatch = parse_ffier_param_dispatch(&pt.attrs)
                    .unwrap_or_else(|| "auto".to_string());
                return Some(ParamInfo {
                    name: pi.ident.clone(),
                    kind: ParamKind::ImplTrait { trait_name, dispatch, trait_lifetime_args },
                    types: Some(TypePair {
                        bridge: quote! { *mut core::ffi::c_void },
                        rust: quote! { *mut core::ffi::c_void },
                    }),
                });
            }

            // Replace Self with concrete type — lifetime-erased for bridge,
            // lifetime-preserving for rust (client codegen needs real lifetimes).
            let param_ty_bridge = match self_ty {
                Some(sty) => {
                    let static_ty = erase_lifetimes(sty);
                    replace_self_type(&pt.ty, &static_ty)
                }
                None => (*pt.ty).clone(),
            };
            let param_ty_rust = match self_ty {
                Some(sty) => replace_self_type(&pt.ty, sty),
                None => (*pt.ty).clone(),
            };

            if is_str_slice(&param_ty_bridge) {
                return Some(ParamInfo {
                    name: pi.ident.clone(),
                    kind: ParamKind::StrSlice,
                    types: None,
                });
            }

            let bridge = ctx.bridge_tokens(&param_ty_bridge);
            Some(ParamInfo {
                name: pi.ident.clone(),
                kind: ParamKind::Regular,
                types: Some(TypePair {
                    bridge,
                    rust: quote! { #param_ty_rust },
                }),
            })
        })
        .collect();

    // --- Parse return type ---
    let self_ty_static = self_ty.map(erase_lifetimes);

    // Builder detection: method returns Self (only for exportable, requires a receiver)
    let has_receiver = receiver != Receiver::None;
    let is_builder = if let (Some(sty_static), true) = (&self_ty_static, has_receiver) {
        match &sig.output {
            ReturnType::Default => false,
            ReturnType::Type(_, ty) => {
                let ty = &replace_self_type(ty, sty_static);
                is_self_return(ty, sty_static)
                    || extract_result_types(ty)
                        .is_some_and(|(ok, _)| is_self_return(&ok, sty_static))
            }
        }
    } else {
        false
    };

    let ret = match &sig.output {
        ReturnType::Default => ReturnKind::Void,
        ReturnType::Type(_, ty) => {
            // Bridge path: Self replaced with lifetime-erased type
            let ty_bridge = match &self_ty_static {
                Some(sty) => replace_self_type(ty, sty),
                None => (**ty).clone(),
            };
            // Rust path: Self replaced with original type (lifetimes preserved)
            let ty_rust = match self_ty {
                Some(sty) => replace_self_type(ty, sty),
                None => (**ty).clone(),
            };

            if is_builder && extract_result_types(&ty_bridge).is_none() {
                // Builder returning Self → void in C
                ReturnKind::Void
            } else if let Some((ok_bridge, err)) = extract_result_types(&ty_bridge) {
                let err_ident = type_ident_name(&err);
                let ok_rust = extract_result_types(&ty_rust).map(|(ok, _)| ok);
                let ok_pair = if is_unit_type(&ok_bridge)
                    || (is_builder && self_ty_static.as_ref().is_some_and(|sty| is_self_return(&ok_bridge, sty)))
                {
                    None
                } else if raw_handle {
                    let erased = erase_lifetimes(&ok_bridge);
                    let rust = ok_rust.as_ref().unwrap_or(&ok_bridge);
                    Some(TypePair {
                        bridge: quote! { #erased },
                        rust: quote! { #rust },
                    })
                } else {
                    let bridge = ctx.bridge_tokens(&ok_bridge);
                    let rust = ok_rust.as_ref().unwrap_or(&ok_bridge);
                    Some(TypePair {
                        bridge,
                        rust: quote! { #rust },
                    })
                };
                ReturnKind::Result { ok: ok_pair, err_ident }
            } else if raw_handle {
                let erased = erase_lifetimes(&ty_bridge);
                ReturnKind::Value(TypePair {
                    bridge: quote! { #erased },
                    rust: quote! { #ty_rust },
                })
            } else {
                let bridge = ctx.bridge_tokens(&ty_bridge);
                ReturnKind::Value(TypePair {
                    bridge,
                    rust: quote! { #ty_rust },
                })
            }
        }
    };

    // --- Original rust_ret for client codegen ---
    let rust_ret = match &sig.output {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => {
            let replaced = match self_ty {
                Some(sty) => replace_self_type(ty, sty),
                None => (**ty).clone(),
            };
            Some(quote! { #replaced })
        }
    };

    Some(MethodInfo {
        name: sig.ident.clone(),
        receiver,
        params,
        ret,
        ffi_name: String::new(),
        is_builder,
        method_lifetimes: sig
            .generics
            .lifetimes()
            .map(|lt| lt.lifetime.ident.clone())
            .collect(),
        doc_lines: extract_doc_comments(attrs),
        rust_ret,
        has_default,
        index: 0,
        raw_handle,
    })
}

/// Emit metadata tokens for a method list (unified format for all annotation macros).
fn emit_method_meta(methods: &[MethodInfo]) -> Vec<proc_macro2::TokenStream> {
    methods.iter().map(emit_one_method_meta).collect()
}

fn emit_one_method_meta(m: &MethodInfo) -> proc_macro2::TokenStream {
    let mname = &m.name;
    let ffi_name = &m.ffi_name;
    let doc_tokens: Vec<_> = m.doc_lines.iter().map(|d| quote! { #d }).collect();

    let receiver_ident = match m.receiver {
        Receiver::None => format_ident!("none"),
        Receiver::Ref => format_ident!("r#ref"),
        Receiver::Mut => format_ident!("r#mut"),
        Receiver::Value => format_ident!("value"),
    };

    let is_builder = m.is_builder;
    let has_default = m.has_default;
    let index = m.index;
    let raw_handle = m.raw_handle;

    let method_lt_idents: Vec<_> = m.method_lifetimes
        .iter()
        .map(|lt| format_ident!("{}", lt))
        .collect();

    let param_tokens: Vec<_> = m.params.iter().map(|p| {
        let id = &p.name;
        let kind_tokens = match &p.kind {
            ParamKind::Regular => quote! { regular },
            ParamKind::StrSlice => quote! { str_slice },
            ParamKind::ImplTrait { trait_name, dispatch, trait_lifetime_args } => {
                let dispatch_ident = format_ident!("{dispatch}");
                let lt_idents: Vec<_> = trait_lifetime_args.iter().map(|lt| format_ident!("{lt}")).collect();
                quote! { impl_trait, trait_name = #trait_name, dispatch = #dispatch_ident, trait_lifetime_args = [#(#lt_idents),*] }
            }
        };
        let type_tokens = match &p.types {
            Some(tp) => {
                let bt = &tp.bridge;
                let rt = &tp.rust;
                quote! { bridge_type = (#bt), rust_type = (#rt), }
            }
            None => quote! {},
        };
        quote! { { name = #id, kind = #kind_tokens, #type_tokens } }
    }).collect();

    let ret_tokens = match &m.ret {
        ReturnKind::Void => quote! { void },
        ReturnKind::Value(tp) => {
            let bt = &tp.bridge;
            let rt = &tp.rust;
            quote! { value(bridge_type = (#bt), rust_type = (#rt),) }
        }
        ReturnKind::Result { ok, err_ident } => {
            let ok_tokens = match ok {
                None => quote! { ok = void },
                Some(tp) => {
                    let bt = &tp.bridge;
                    let rt = &tp.rust;
                    quote! { ok = some(bridge_type = (#bt), rust_type = (#rt),) }
                }
            };
            quote! { result(#ok_tokens, err_ident = #err_ident,) }
        }
    };

    let rust_ret_tokens = match &m.rust_ret {
        Some(rt) => quote! { rust_ret = (#rt), },
        None => quote! { rust_ret = (()), },
    };

    quote! {
        {
            name = #mname,
            ffi_name = #ffi_name,
            doc = [#(#doc_tokens),*],
            receiver = #receiver_ident,
            is_builder = #is_builder,
            has_default = #has_default,
            index = #index,
            raw_handle = #raw_handle,
            method_lifetimes = [#(#method_lt_idents),*],
            params = [#(#param_tokens),*],
            ret = #ret_tokens,
            #rust_ret_tokens
        }
    }
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
    let is_foreign = args.foreign;
    let trait_item = parse_macro_input!(item as ItemTrait);
    let original_trait = trait_item.clone();

    let trait_name = &trait_item.ident;
    let trait_name_str = trait_name.to_string();
    let trait_snake = camel_to_snake(&trait_name_str);

    // Extract trait-level lifetime params (e.g. `<'a>` from `trait Snapshot<'a>`)
    let trait_lifetime_idents: Vec<&syn::Ident> = trait_item
        .generics
        .lifetimes()
        .map(|lt| &lt.lifetime.ident)
        .collect();

    let vtable_struct_name = format_ident!("{trait_name_str}Vtable");

    let wrapper_name = format_ident!("Vtable{trait_name_str}");
    let wrapper_c_handle_suffix = format!("Vtable{trait_name_str}");

    let helper_mod_name = format_ident!("_ffier_vtable_{trait_snake}");
    let mut ctx = AliasContext::new(helper_mod_name.clone());

    // Extract all methods (trait + supertraits).
    // own_method_count tracks how many belong to this trait (before supers).
    let vtable_methods =
        match extract_vtable_methods(&trait_item, &args.supers, &args.reserved, &mut ctx) {
            Ok(v) => v,
            Err(e) => return e.to_compile_error().into(),
        };
    // Count own methods (excluding supertrait methods which are appended
    // by extract_vtable_methods after the trait's own methods).
    let super_method_count: usize = args.supers.iter().map(|s| s.methods.len()).sum();
    let own_method_count = vtable_methods.len() - super_method_count;

    // --- Generate vtable struct fields (ordered by explicit index, with padding for gaps) ---
    // Sort methods by their explicit index to determine vtable layout.
    let mut sorted_methods: Vec<&MethodInfo> = vtable_methods.iter().collect();
    sorted_methods.sort_by_key(|m| m.index);

    let mut vtable_fields: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut next_slot = 0usize;

    for m in &sorted_methods {
        // raw_handle methods don't occupy vtable slots — they're dispatched
        // via the bridge directly (composing other trait methods).
        if m.raw_handle {
            continue;
        }

        // Insert padding fields for any gaps
        while next_slot < m.index {
            let pad_name = format_ident!("__reserved_{next_slot}");
            vtable_fields.push(quote! {
                #[doc(hidden)]
                pub #pad_name: Option<unsafe extern "C" fn()>
            });
            next_slot += 1;
        }

        let name = &m.name;
        let params: Vec<_> = m
            .params
            .iter()
            .map(|p| {
                if p.is_impl_trait() {
                    // impl Trait → raw handle pointer in the vtable fn signature
                    quote! { *mut core::ffi::c_void }
                } else {
                    let bt = p.bridge_type();
                    quote! { <#bt as ffier::FfiType>::CRepr }
                }
            })
            .collect();
        let ret = match m.ret_bridge_type() {
            None => quote! {},
            Some(bt) => quote! { -> <#bt as ffier::FfiType>::CRepr },
        };
        vtable_fields.push(quote! {
            pub #name: Option<unsafe extern "C" fn(*mut core::ffi::c_void, #(#params),*) #ret>
        });
        next_slot = m.index + 1;
    }

    // Add trailing padding for reserved slots beyond the last method index.
    // Gaps between methods are already handled by the loop above; here we only
    // need to extend the vtable for reserved indices that fall after all methods.
    if let Some(&max_reserved) = args.reserved.iter().max() {
        while next_slot <= max_reserved {
            let pad_name = format_ident!("__reserved_{next_slot}");
            vtable_fields.push(quote! {
                #[doc(hidden)]
                pub #pad_name: Option<unsafe extern "C" fn()>
            });
            next_slot += 1;
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
    // Erase lifetimes only on trait-level generics, not on method parameter types.
    // Method signatures keep their original lifetimes (anonymous `'_` for input
    // positions like `s: &str`) — erasing them to `'static` would make the impl
    // unsatisfiable for callers with shorter borrows.
    let trait_item_erased = {
        let mut t = trait_item.clone();
        struct Eraser;
        impl VisitMut for Eraser {
            fn visit_lifetime_mut(&mut self, lt: &mut syn::Lifetime) {
                *lt = syn::Lifetime::new("'static", lt.apostrophe);
            }
            // Don't descend into method signatures — keep their lifetimes as-is.
            fn visit_trait_item_fn_mut(&mut self, _: &mut syn::TraitItemFn) {}
        }
        Eraser.visit_item_trait_mut(&mut t);
        t
    };

    // Helper: generate the vtable call expression for a method (unwrapping Option).
    // `fallback` is Some(tokens) for defaulted methods, None for required methods.
    //
    // For defaulted methods, we first check the metadata field on the handle:
    // if bit 0 is set and the method index matches, we skip vtable dispatch and
    // call the library default directly. This prevents infinite re-entrancy when
    // a client trait default calls through self-dispatch.
    // Helper: generate the vtable call expression for a method.
    // `vtable_struct_ref` is the token stream referencing the vtable struct
    // (e.g. `PushStrVtable` for direct output, or `$crate::PushStrVtable`
    // for use inside a macro_rules! body that expands in another crate).
    let vtable_call_body = |vm: &MethodInfo,
                            sig: &syn::Signature,
                            fallback: Option<proc_macro2::TokenStream>,
                            _method_index: Option<usize>,
                            vtable_struct_ref: &proc_macro2::TokenStream|
     -> proc_macro2::TokenStream {
        let name = &vm.name;
        let name_str = name.to_string();
        // Check if any param is impl Trait — vtable dispatch through C
        // function pointers can't handle impl Trait params generically
        // (the value might not be an FFI handle). If so, the vtable branch
        // panics at runtime.
        let has_impl_trait_params = vm.params.iter().any(|p| p.is_impl_trait());

        let vtable_args: Vec<_> = vm
            .params
            .iter()
            .map(|p| {
                let id = &p.name;
                if p.is_impl_trait() {
                    // Placeholder — won't actually be reached due to the panic
                    // guard below, but needed for the code to compile.
                    quote! { core::ptr::null_mut() }
                } else {
                    // Use rust_type (elided lifetimes) for the conversion call,
                    // not bridge_type ('static lifetimes). The actual value has
                    // the caller's lifetime, not 'static.
                    let rt = p.rust_type();
                    quote! { <#rt as ffier::FfiType>::into_c(#id) }
                }
            })
            .collect();
        // Build the concrete fn pointer type for this vtable field
        let param_bridge_types: Vec<_> = vm
            .params
            .iter()
            .map(|p| {
                if p.is_impl_trait() {
                    // impl Trait → *mut c_void in the function pointer signature
                    quote! { *mut core::ffi::c_void }
                } else {
                    let bt = p.bridge_type();
                    quote! { <#bt as ffier::FfiType>::CRepr }
                }
            })
            .collect();
        let fn_ret = match vm.ret_bridge_type() {
            None => quote! {},
            Some(bt) => quote! { -> <#bt as ffier::FfiType>::CRepr },
        };
        let fn_ptr_type = quote! {
            unsafe extern "C" fn(*mut core::ffi::c_void #(, #param_bridge_types)*) #fn_ret
        };

        let raw_call = quote! {
            unsafe { __f(self.value.user_data as *mut core::ffi::c_void, #(#vtable_args),*) }
        };
        let vtable_branch = if has_impl_trait_params {
            // Methods with impl Trait params can't dispatch through C vtable
            // function pointers — the generic param isn't necessarily an FFI
            // handle. Panic if this path is ever reached at runtime.
            let wrapper_str = wrapper_name.to_string();
            quote! {
                let _ = __f;
                panic!(
                    "{}: vtable dispatch for method `{}` with impl Trait params is not supported",
                    #wrapper_str, #name_str,
                )
            }
        } else {
            match vm.ret_bridge_type() {
                None => raw_call,
                Some(bt) => quote! { <#bt as ffier::FfiType>::from_c(#raw_call) },
            }
        };
        let none_branch = match &fallback {
            Some(fb) => fb.clone(),
            None => {
                let wrapper_str = wrapper_name.to_string();
                quote! {
                    panic!(
                        "{}: required vtable method `{}` not provided",
                        #wrapper_str, #name_str,
                    )
                }
            }
        };
        // No metadata check inside VtableFoo::method() — the metadata field
        // lives outside the provenance of `&self`. The self-dispatch function
        // reads metadata from the raw handle pointer (which has full provenance
        // over the entire handle) and skips vtable dispatch if the metadata
        // indicates a default-method dispatch skip.
        let metadata_check = quote! {};
        quote! {
            #sig {
                #metadata_check
                let __field: Option<#fn_ptr_type> = unsafe {
                    self.value.field_or_none(
                        core::mem::offset_of!(#vtable_struct_ref, #name),
                    )
                };
                match __field {
                    Some(__f) => { #vtable_branch }
                    None => { #none_branch }
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
    impl SelfReplacer {
        /// Replace `self` idents in a raw TokenStream (for macro bodies
        /// that syn::visit_mut doesn't traverse).
        fn replace_in_token_stream(
            &self,
            ts: proc_macro2::TokenStream,
        ) -> proc_macro2::TokenStream {
            ts.into_iter()
                .map(|tt| match tt {
                    proc_macro2::TokenTree::Ident(ref id) if id == "self" => {
                        proc_macro2::TokenTree::Ident(proc_macro2::Ident::new("__self", id.span()))
                    }
                    proc_macro2::TokenTree::Group(g) => {
                        let inner = self.replace_in_token_stream(g.stream());
                        let mut new_g = proc_macro2::Group::new(g.delimiter(), inner);
                        new_g.set_span(g.span());
                        proc_macro2::TokenTree::Group(new_g)
                    }
                    other => other,
                })
                .collect()
        }
    }
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
        fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
            mac.tokens = self.replace_in_token_stream(mac.tokens.clone());
        }
    }

    // Foreign traits must not have default method bodies — the real defaults
    // live in the foreign crate and can't be replicated here.
    // Exception: raw_handle methods always need their default body because
    // they're not dispatched through the vtable.
    if is_foreign {
        for item in &trait_item.items {
            if let TraitItem::Fn(method) = item {
                let is_raw_handle = vtable_methods
                    .iter()
                    .any(|vm| vm.name == method.sig.ident && vm.raw_handle);
                if method.default.is_some() && !is_raw_handle {
                    return syn::Error::new_spanned(
                        &method.sig.ident,
                        "foreign trait methods must not have default bodies \
                         (the defaults live in the foreign crate)",
                    )
                    .to_compile_error()
                    .into();
                }
            }
        }
    }

    let mut default_helpers: Vec<proc_macro2::TokenStream> = Vec::new();
    // Map from method name → helper fn ident (only for methods with defaults)
    let mut default_helper_names: HashMap<String, syn::Ident> = HashMap::new();

    // Build modified trait: replace default bodies with helper calls
    // and strip #[ffier(index = N)] attributes (consumed by macro).
    let mut modified_trait = original_trait.clone();
    for item in &mut modified_trait.items {
        let TraitItem::Fn(method) = item else {
            continue;
        };
        // Strip all #[ffier(...)] from emitted method attrs (consumed by this macro)
        let is_raw_handle = vtable_methods
            .iter()
            .any(|vm| vm.name == method.sig.ident && vm.raw_handle);
        method.attrs.retain(|attr| !attr.path().is_ident("ffier"));
        let Some(default_block) = &method.default else {
            continue;
        };
        // raw_handle methods keep their default body as-is — they don't
        // go through the vtable helper extraction, and the VtableWrapper
        // uses the trait's default impl directly.
        if is_raw_handle {
            continue;
        }

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
            pub #helper_sig #body
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
    // These are used inside the @reexport macro arm, so references to items
    // in the defining crate use $crate:: prefix. This ensures correct resolution
    // when the macro expands in a different crate.
    let vtable_struct_ref = quote! { $crate::#vtable_struct_name };
    let own_method_impls: Vec<_> = trait_item_erased
        .items
        .iter()
        .filter_map(|item| {
            let TraitItem::Fn(method) = item else {
                return None;
            };
            let name = &method.sig.ident;
            let vm = vtable_methods.iter().find(|v| v.name == *name)?;
            // Skip raw_handle methods — they don't take &self, so the
            // VtableWrapper can't override them. The trait's default impl
            // handles dispatch (it calls other trait methods via &self).
            if vm.raw_handle {
                return None;
            }
            // Use the explicit index for metadata dispatch
            let method_index = Some(vm.index);
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
                // Use $crate:: prefix so the helper resolves in the defining crate
                // when @reexport expands in a different crate.
                quote! { $crate::#helper(self #(, #params_pass)*) }
            });
            Some(vtable_call_body(vm, &method.sig, fallback, method_index, &vtable_struct_ref))
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
                    let vm = vtable_methods.iter().find(|v| v.name == *name)?;
                    Some(vtable_call_body(vm, &method.sig, None, None, &vtable_struct_ref))
                })
                .collect();

            // Use $crate:: for supertrait path so it resolves in the defining crate.
            quote! {
                impl $crate::#tn for #wrapper_name {
                    #(#method_impls)*
                }
            }
        })
        .collect();

    // --- Compute max vtable slot for metadata ---
    // The highest slot index that needs to be padded in the vtable struct.
    // This accounts for both method indices and reserved (retired) slots.
    let max_method_index = vtable_methods.iter().map(|m| m.index).max();
    let max_reserved_index = args.reserved.iter().copied().max();
    let max_vtable_slot_val: usize = match (max_method_index, max_reserved_index) {
        (Some(mi), Some(ri)) => mi.max(ri),
        (Some(v), None) | (None, Some(v)) => v,
        (None, None) => 0,
    };

    // --- Metadata emission ---
    let counter = MACRO_COUNTER.fetch_add(1, Ordering::SeqCst);
    let internal_macro_name = format_ident!("__ffier_internal_{trait_snake}_{counter}");
    let meta_alias_name = format_ident!("__ffier_meta_{trait_name}");


    let vtable_method_meta = emit_method_meta(&vtable_methods);

    let trait_path_tokens = quote! { $crate::#trait_name };

    let reexport_items_crate = ctx.reexport_items_crate();

    // Generate FfierBoxDyn delegation (implies #[ffier::dispatch])
    // For traits with supertraits, we also need to delegate the supertrait methods.
    // Skip for foreign traits (orphan rules: can't impl foreign trait for local type).
    let boxdyn_impl = if is_foreign {
        quote! {}
    } else {
        let has_supertraits = !args.supers.is_empty()
            || trait_item
                .supertraits
                .iter()
                .any(|b| matches!(b, syn::TypeParamBound::Trait(_)));
        if !has_supertraits {
            emit_boxdyn_impl(&trait_item)
        } else {
            // TODO: generate supertrait delegation for FfierBoxDyn
            quote! {}
        }
    };

    // For foreign traits (`extern trait`), don't re-emit the trait definition —
    // it already exists in the foreign crate. Only emit the generated plumbing.
    let trait_tokens = if is_foreign {
        quote! {}
    } else {
        quote! { #modified_trait }
    };

    let output = quote! {
        #(#default_helpers)*

        #trait_tokens

        #boxdyn_impl

        #[repr(C)]
        pub struct #vtable_struct_name {
            /// Drop callback. Always the first field so its offset is stable
            /// across vtable versions (forward-compatible ABI).
            pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
            #(#vtable_fields,)*
        }

        #[doc(hidden)]
        #[macro_export]
        macro_rules! #internal_macro_name {
            // @reexport: generates the wrapper type, its trait impl, Drop,
            // FfiHandle, and FfiType impls. Called by library_definition! with
            // the type tag.
            //
            // The wrapper type is emitted at the crate root of the invoking
            // crate, so orphan rules are satisfied even when the trait is
            // defined in an upstream crate.
            //
            // The vtable struct and trait must be re-exported at the library
            // crate root so that the shim's path overrides resolve correctly.
            (@reexport, $type_tag:expr) => {
                #[doc(hidden)]
                pub mod #helper_mod_name {
                    #(#reexport_items_crate)*
                }

                /// Wrapper type for vtable-dispatched trait implementations.
                #[repr(C)]
                pub struct #wrapper_name {
                    pub value: ffier::VtableHandle,
                }

                impl $crate::#trait_name #trait_ty_generics for #wrapper_name {
                    #(#own_method_impls)*
                }

                #(#super_impls)*

                impl Drop for #wrapper_name {
                    fn drop(&mut self) {
                        let drop_field: Option<unsafe extern "C" fn(*mut core::ffi::c_void)> = unsafe {
                            self.value.field_or_none(
                                core::mem::offset_of!($crate::#vtable_struct_name, drop),
                            )
                        };
                        if let Some(drop_fn) = drop_field {
                            unsafe { drop_fn(self.value.user_data as *mut core::ffi::c_void) };
                        }
                    }
                }

                impl ffier::FfiHandle for #wrapper_name {
                    const C_HANDLE_NAME: &str = #wrapper_c_handle_suffix;
                    const TYPE_TAG: u32 = $type_tag;
                    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
                        let ptr = (self as *const Self as *const u8)
                            .wrapping_sub(ffier::HANDLE_VALUE_OFFSET);
                        ptr as *mut core::ffi::c_void
                    }
                }

                impl ffier::FfiType for #wrapper_name {
                    type CRepr = *mut core::ffi::c_void;
                    const C_TYPE_NAME: &str = #wrapper_c_handle_suffix;
                    const IS_HANDLE: bool = true;
                    fn into_c(self) -> *mut core::ffi::c_void {
                        ffier::ffier_handle_new(
                            <Self as ffier::FfiHandle>::TYPE_TAG,
                            self,
                        )
                    }
                    fn from_c(repr: *mut core::ffi::c_void) -> Self {
                        unsafe { ffier::ffier_handle_consume::<Self>(repr) }
                    }
                }

            };
            // Tagged invocation with path overrides (from library_definition!
            // shim for trait entries): the shim passes wrapper_name,
            // vtable_struct, and trait_path as parenthesized groups so the
            // metadata blob uses the library crate's paths for cross-crate
            // traits. Downstream crates in the chain only need the library
            // crate, not the upstream crate that defined #[implementable].
            ($prefix:literal, $type_tag:expr,
             ($($wrapper:tt)*), ($($vstruct:tt)*), ($($tpath:tt)*),
             $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @implementable,
                    trait_name = #trait_name,
                    trait_path = ($($tpath)*),
                    prefix = $prefix,
                    type_tag = $type_tag,
                    vtable_struct = ($($vstruct)*),
                    wrapper_name = ($($wrapper)*),
                    trait_lifetimes = (#(#trait_lifetime_idents),*),
                    vtable_methods = [#(#vtable_method_meta),*],
                    own_method_count = #own_method_count,
                    max_vtable_slot = #max_vtable_slot_val,
                } $(, $($rest)*)? }
            };
            // Tagged invocation (legacy, same-crate): wrapper_name defaults to
            // $crate::VtableFoo. Still works for same-crate traits but NOT for
            // cross-crate (wrapper is now generated by @reexport in the user's crate).
            ($prefix:literal, $type_tag:expr, $callback:path $(, $($rest:tt)*)?) => {
                $callback! { {
                    @implementable,
                    trait_name = #trait_name,
                    trait_path = (#trait_path_tokens),
                    prefix = $prefix,
                    type_tag = $type_tag,
                    vtable_struct = ($crate::#vtable_struct_name),
                    wrapper_name = ($crate::#wrapper_name),
                    trait_lifetimes = (#(#trait_lifetime_idents),*),
                    vtable_methods = [#(#vtable_method_meta),*],
                    own_method_count = #own_method_count,
                    max_vtable_slot = #max_vtable_slot_val,
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
                    trait_lifetimes = (#(#trait_lifetime_idents),*),
                    vtable_methods = [#(#vtable_method_meta),*],
                    own_method_count = #own_method_count,
                    max_vtable_slot = #max_vtable_slot_val,
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
            method.attrs.retain(|a| !a.path().is_ident("ffier"));
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
    let methods: Vec<MethodInfo> = {
        let mut ms = Vec::new();
        for item in &input.items {
            let ImplItem::Fn(method) = item else { continue };
            let mattrs = match parse_ffier_method_attrs(&method.attrs) {
                Ok(a) => a,
                Err(e) => return e.to_compile_error().into(),
            };
            if mattrs.skip { continue; }
            // trait_impl methods are concrete overrides, not defaults
            if let Some(m) = parse_method_sig(&method.sig, &method.attrs, &mut ctx, None, false, mattrs.raw_handle) {
                ms.push(m);
            }
        }
        ms
    };

    let reexport_items_crate = ctx.reexport_items_crate();

    let method_meta = emit_method_meta(&methods);

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
/// mylib::__ffier_mylib_library!(ffier_bridge_macros::generate);
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

                // The upstream metadata macro generates the wrapper type
                // via @reexport. The shim passes path overrides (wrapper,
                // vtable struct, trait path) using $crate:: which resolves
                // to the library crate. Downstream crates in the chain only
                // need the library crate, not the upstream crate.
                let alias = meta_alias_for_type(path);
                let wrapper_ident = format_ident!("Vtable{last_ident}");
                let vtable_struct_ident = format_ident!("{last_ident}Vtable");

                // Re-export the upstream metadata macro so the shim can
                // call it via $crate:: (library crate path). This way
                // downstream crates don't need the upstream crate as a dep.
                let upstream_alias = format_ident!("__ffier_upstream_{last_ident}");
                reexport_invocations.push(quote! {
                    #[doc(hidden)]
                    pub use #alias as #upstream_alias;
                });

                // Re-export the trait and vtable struct so `$crate::PushStr`
                // and `$crate::PushStrVtable` resolve without manual `pub use`
                // at the library crate root. Only needed for external crate
                // paths (not `crate::` which is already local).
                // Single-segment paths (like `Processor`) and `crate::` paths
                // are local — they're already defined in the library crate.
                // Multi-segment external paths (like `ffier_builtins::PushStr`)
                // need re-exporting.
                let is_external = path.segments.len() > 1
                    && path.segments.first().map_or(true, |seg| seg.ident != "crate");
                if is_external {
                    let vtable_struct_path = replace_last_segment(path, &vtable_struct_ident);
                    reexport_invocations.push(quote! {
                        #[doc(hidden)]
                        pub use #path;
                        #[doc(hidden)]
                        pub use #vtable_struct_path;
                    });
                }

                let shim_name = format_ident!("__ffier_tagged_{prefix_str}_{last_ident}");
                shim_macros.push(quote! {
                    #[doc(hidden)]
                    #[macro_export]
                    macro_rules! #shim_name {
                        ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                            $crate::#upstream_alias! { $prefix, #tag,
                                ($crate::#wrapper_ident),
                                ($crate::#vtable_struct_ident),
                                ($crate::#last_ident),
                                $callback $(, $($rest)*)? }
                        };
                    }
                });

                // @reexport with type_tag generates the wrapper type + impls.
                reexport_invocations.push(quote! { #alias!(@reexport, #tag); });
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
