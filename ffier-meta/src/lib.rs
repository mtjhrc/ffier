//! Metadata types for ffier's reflection-based architecture.
//!
//! `#[ffier::exportable]` emits a metadata macro containing structured tokens.
//! Generator proc macros (`generate`) parse
//! these tokens back into the types defined here, then produce code.

use proc_macro2::TokenStream;
use syn::{Ident, LitBool, LitStr, Token, parse::ParseStream};

// ---------------------------------------------------------------------------
// String case conversion helpers
// ---------------------------------------------------------------------------

pub fn camel_to_snake(s: &str) -> String {
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

pub fn camel_to_upper_snake(s: &str) -> String {
    camel_to_snake(s).to_ascii_uppercase()
}

pub fn snake_to_pascal(s: &str) -> String {
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

// ---------------------------------------------------------------------------
// Peek helper
// ---------------------------------------------------------------------------

/// Peek at the `@tag` identifier from a metadata token stream without consuming it.
pub fn peek_meta_tag(input: &TokenStream) -> String {
    let mut iter = input.clone().into_iter();
    // Skip `@` punct
    if let Some(proc_macro2::TokenTree::Punct(p)) = iter.next()
        && p.as_char() == '@'
        && let Some(proc_macro2::TokenTree::Ident(id)) = iter.next()
    {
        return id.to_string();
    }
    String::new()
}

/// Peek at the type/trait name from a metadata token stream.
///
/// Looks for `name = IDENT` or `trait_name = IDENT` and returns the IDENT.
pub fn peek_meta_name(input: &TokenStream) -> String {
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

/// Peek at a specific `field = VALUE` from a metadata token stream.
/// VALUE can be an ident or a string literal.
pub fn peek_meta_field(input: &TokenStream, field: &str) -> String {
    let tokens: Vec<proc_macro2::TokenTree> = input.clone().into_iter().collect();
    for i in 0..tokens.len().saturating_sub(2) {
        if let proc_macro2::TokenTree::Ident(ref id) = tokens[i]
            && id == field
            && let proc_macro2::TokenTree::Punct(ref p) = tokens[i + 1]
            && p.as_char() == '='
        {
            // The value might be wrapped in a Delimiter::None group
            // (macro_rules! replayed captures produce these).
            let val = match &tokens[i + 2] {
                proc_macro2::TokenTree::Group(g)
                    if g.delimiter() == proc_macro2::Delimiter::None =>
                {
                    g.stream().into_iter().next()
                }
                other => Some(other.clone()),
            };
            match val {
                Some(proc_macro2::TokenTree::Ident(name)) => return name.to_string(),
                Some(proc_macro2::TokenTree::Literal(lit)) => {
                    let s = lit.to_string();
                    return s.trim_matches('"').to_string();
                }
                _ => {}
            }
        }
    }
    "Unknown".to_string()
}

// ---------------------------------------------------------------------------
// Metadata types --- parsed from the metadata macro's token stream
// ---------------------------------------------------------------------------

pub struct MetaExportable {
    pub struct_name: Ident,
    pub struct_path: TokenStream,
    pub prefix: String,
    pub lifetimes: Vec<Ident>,
    pub type_aliases: Vec<(Ident, TokenStream)>,
    pub methods: Vec<MetaMethod>,
}

impl MetaExportable {
    pub fn fn_pfx(&self) -> String {
        format!("{}_", self.prefix)
    }

    pub fn type_pfx(&self) -> String {
        snake_to_pascal(&self.prefix)
    }

    pub fn upper_pfx(&self) -> String {
        format!("{}_", self.prefix.to_ascii_uppercase())
    }

    pub fn handle_c_name(&self) -> String {
        format!("{}{}", self.type_pfx(), self.struct_name)
    }

    pub fn uses_slices(&self) -> bool {
        // With trait-based type mapping, we can't detect slice usage at parse
        // time. Always emit the shared types — unused typedefs are harmless.
        true
    }
}

pub struct MetaMethod {
    pub name: Ident,
    pub ffi_name: String,
    pub doc: Vec<String>,
    pub receiver: MetaReceiver,
    pub is_builder: bool,
    /// Method-level lifetime params (e.g. `[a, b]` from `fn foo<'a, 'b>(...)`).
    pub method_lifetimes: Vec<Ident>,
    pub params: Vec<MetaParam>,
    pub ret: MetaReturn,
    pub rust_ret: TokenStream,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MetaReceiver {
    None,
    Ref,
    Mut,
    Value,
}

pub struct MetaParam {
    pub name: Ident,
    pub kind: MetaParamKind,
    pub rust_type: Option<TokenStream>,
}

pub enum MetaParamKind {
    Regular {
        bridge_type: TokenStream,
    },
    StrSlice,
    DynDispatch {
        c_name_suffix: String,
        variants: Vec<(String, TokenStream)>,
    },
}

pub enum MetaReturn {
    Void,
    Value(MetaValueKind),
    Result {
        ok: Option<MetaValueKind>,
        #[allow(dead_code)]
        err_bridge_type: TokenStream,
        err_ident: String,
    },
}

pub enum MetaValueKind {
    Regular {
        bridge_type: TokenStream,
    },
}

// ---------------------------------------------------------------------------
// Error metadata
// ---------------------------------------------------------------------------

pub struct MetaError {
    pub name: Ident,
    pub path: TokenStream,
    pub prefix: String,
    pub variants: Vec<MetaErrorVariant>,
}

impl MetaError {
    pub fn fn_pfx(&self) -> String {
        format!("{}_", self.prefix)
    }

    pub fn type_pfx(&self) -> String {
        snake_to_pascal(&self.prefix)
    }

    pub fn upper_pfx(&self) -> String {
        format!("{}_", self.prefix.to_ascii_uppercase())
    }
}

pub struct MetaErrorVariant {
    pub name: Ident,
    pub code: u64,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Implementable metadata
// ---------------------------------------------------------------------------

pub struct MetaImplementable {
    pub trait_name: Ident,
    pub trait_path: TokenStream,
    pub prefix: String,
    pub vtable_struct_name: TokenStream,
    pub wrapper_name: TokenStream,
    pub vtable_fields: Vec<MetaVtableField>,
    pub vtable_methods: Vec<MetaVtableMethod>,
}

impl MetaImplementable {
    pub fn fn_pfx(&self) -> String {
        format!("{}_", self.prefix)
    }

    pub fn type_pfx(&self) -> String {
        snake_to_pascal(&self.prefix)
    }

    pub fn vtable_c_name(&self) -> String {
        format!("{}{}Vtable", self.type_pfx(), self.trait_name)
    }

    pub fn constructor_name(&self) -> String {
        format!(
            "{}{}_from_vtable",
            self.fn_pfx(),
            camel_to_snake(&self.trait_name.to_string())
        )
    }
}

pub struct MetaVtableField {
    pub name: Ident,
    pub field_type: TokenStream,
}

pub struct MetaVtableMethod {
    pub name: Ident,
    pub params: Vec<MetaVtableParam>,
    pub ret: MetaVtableRet,
}

pub struct MetaVtableParam {
    pub name: Ident,
    pub bridge_type: TokenStream,
    pub rust_type: TokenStream,
}

pub enum MetaVtableRet {
    Void,
    Value {
        bridge_type: TokenStream,
        rust_type: TokenStream,
    },
}

// ---------------------------------------------------------------------------
// Trait impl metadata (impl Trait for Struct, exported via C ABI)
// ---------------------------------------------------------------------------

pub struct MetaTraitImpl {
    pub trait_name: Ident,
    pub struct_name: Ident,
    pub struct_path: TokenStream,
    pub trait_path: TokenStream,
    pub prefix: String,
    /// Lifetime params from the impl block (e.g. `[a]` from `impl<'a> Trait<'a> for Struct<'a>`).
    pub lifetimes: Vec<Ident>,
    /// Lifetime arguments on the trait (e.g. `["static"]` from `impl Trait<'static> for Struct`,
    /// or `["a"]` from `impl<'a> Trait<'a> for Struct<'a>`). May differ from `lifetimes`.
    pub trait_lifetime_args: Vec<String>,
    pub methods: Vec<MetaVtableMethod>,
}

impl MetaTraitImpl {
    pub fn fn_pfx(&self) -> String {
        format!("{}_", self.prefix)
    }

    pub fn type_pfx(&self) -> String {
        snake_to_pascal(&self.prefix)
    }
}

// ---------------------------------------------------------------------------
// Parsing --- from token stream back to metadata types
// ---------------------------------------------------------------------------

// Helper: parse `key = value` where key must match expected
fn expect_key(input: ParseStream, expected: &str) -> syn::Result<()> {
    let key: Ident = input.parse()?;
    if key != expected {
        return Err(syn::Error::new(
            key.span(),
            format!("expected `{expected}`, got `{key}`"),
        ));
    }
    input.parse::<Token![=]>()?;
    Ok(())
}

fn parse_comma(input: ParseStream) -> syn::Result<()> {
    if !input.is_empty() && input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    }
    Ok(())
}

fn parse_parenthesized_tokens(input: ParseStream) -> syn::Result<TokenStream> {
    let content;
    syn::parenthesized!(content in input);
    content.parse()
}

fn parse_bool(input: ParseStream) -> syn::Result<bool> {
    let lit: LitBool = input.parse()?;
    Ok(lit.value())
}

fn parse_string(input: ParseStream) -> syn::Result<String> {
    let lit: LitStr = input.parse()?;
    Ok(lit.value())
}

impl syn::parse::Parse for MetaExportable {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // @exportable
        input.parse::<Token![@]>()?;
        let tag: Ident = input.parse()?;
        if tag != "exportable" {
            return Err(syn::Error::new(tag.span(), "expected `exportable`"));
        }
        parse_comma(input)?;

        expect_key(input, "name")?;
        let struct_name: Ident = input.parse()?;
        parse_comma(input)?;

        expect_key(input, "struct_path")?;
        let struct_path = parse_parenthesized_tokens(input)?;
        parse_comma(input)?;

        expect_key(input, "prefix")?;
        let prefix = parse_string(input)?;
        parse_comma(input)?;

        expect_key(input, "lifetimes")?;
        let lifetimes = {
            let content;
            syn::parenthesized!(content in input);
            let mut lts = Vec::new();
            while !content.is_empty() {
                lts.push(content.parse::<Ident>()?);
                if !content.is_empty() {
                    content.parse::<Token![,]>()?;
                }
            }
            lts
        };
        parse_comma(input)?;

        expect_key(input, "type_aliases")?;
        let type_aliases = {
            let content;
            syn::bracketed!(content in input);
            let mut aliases = Vec::new();
            while !content.is_empty() {
                let inner;
                syn::parenthesized!(inner in content);
                let alias: Ident = inner.parse()?;
                inner.parse::<Token![,]>()?;
                let path: TokenStream = inner.parse()?;
                aliases.push((alias, path));
                if !content.is_empty() && content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
            aliases
        };
        parse_comma(input)?;

        expect_key(input, "methods")?;
        let methods = {
            let content;
            syn::bracketed!(content in input);
            let mut ms = Vec::new();
            while !content.is_empty() {
                ms.push(content.parse::<MetaMethod>()?);
                if !content.is_empty() && content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
            ms
        };
        parse_comma(input)?;

        Ok(MetaExportable {
            struct_name,
            struct_path,
            prefix,
            lifetimes,
            type_aliases,
            methods,
        })
    }
}

impl syn::parse::Parse for MetaMethod {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::braced!(content in input);
        let input = &content;

        expect_key(input, "name")?;
        let name: Ident = input.parse()?;
        parse_comma(input)?;

        expect_key(input, "ffi_name")?;
        let ffi_name = parse_string(input)?;
        parse_comma(input)?;

        expect_key(input, "doc")?;
        let doc = {
            let inner;
            syn::bracketed!(inner in input);
            let mut docs = Vec::new();
            while !inner.is_empty() {
                docs.push(parse_string(&inner)?);
                if !inner.is_empty() && inner.peek(Token![,]) {
                    inner.parse::<Token![,]>()?;
                }
            }
            docs
        };
        parse_comma(input)?;

        expect_key(input, "receiver")?;
        let receiver = {
            let r: Ident = input.parse()?;
            match r.to_string().as_str() {
                "none" => MetaReceiver::None,
                "r#ref" | "ref" => MetaReceiver::Ref,
                "r#mut" | "mut" => MetaReceiver::Mut,
                "value" => MetaReceiver::Value,
                other => {
                    return Err(syn::Error::new(
                        r.span(),
                        format!("unknown receiver `{other}`"),
                    ));
                }
            }
        };
        parse_comma(input)?;

        expect_key(input, "is_builder")?;
        let is_builder = parse_bool(input)?;
        parse_comma(input)?;

        expect_key(input, "method_lifetimes")?;
        let method_lifetimes = {
            let inner;
            syn::bracketed!(inner in input);
            let mut lts = Vec::new();
            while !inner.is_empty() {
                lts.push(inner.parse::<Ident>()?);
                if !inner.is_empty() && inner.peek(Token![,]) {
                    inner.parse::<Token![,]>()?;
                }
            }
            lts
        };
        parse_comma(input)?;

        expect_key(input, "params")?;
        let params = {
            let inner;
            syn::bracketed!(inner in input);
            let mut ps = Vec::new();
            while !inner.is_empty() {
                ps.push(inner.parse::<MetaParam>()?);
                if !inner.is_empty() && inner.peek(Token![,]) {
                    inner.parse::<Token![,]>()?;
                }
            }
            ps
        };
        parse_comma(input)?;

        expect_key(input, "ret")?;
        let ret = input.parse::<MetaReturn>()?;
        parse_comma(input)?;

        expect_key(input, "rust_ret")?;
        let rust_ret = parse_parenthesized_tokens(input)?;
        parse_comma(input)?;

        Ok(MetaMethod {
            name,
            ffi_name,
            doc,
            receiver,
            is_builder,
            method_lifetimes,
            params,
            ret,
            rust_ret,
        })
    }
}

impl syn::parse::Parse for MetaParam {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        syn::braced!(content in input);
        let input = &content;

        expect_key(input, "name")?;
        let name: Ident = input.parse()?;
        parse_comma(input)?;

        expect_key(input, "kind")?;
        let kind_ident: Ident = input.parse()?;
        let kind = match kind_ident.to_string().as_str() {
            "regular" => {
                parse_comma(input)?;
                expect_key(input, "bridge_type")?;
                let bridge_type = parse_parenthesized_tokens(input)?;
                MetaParamKind::Regular { bridge_type }
            }
            "str_slice" => MetaParamKind::StrSlice,
            "dyn_dispatch" => {
                parse_comma(input)?;
                expect_key(input, "c_name_suffix")?;
                let c_name_suffix = parse_string(input)?;
                parse_comma(input)?;
                expect_key(input, "variants")?;
                let variants = {
                    let inner;
                    syn::bracketed!(inner in input);
                    let mut vs = Vec::new();
                    while !inner.is_empty() {
                        let vinner;
                        syn::parenthesized!(vinner in inner);
                        let vname = parse_string(&vinner)?;
                        vinner.parse::<Token![,]>()?;
                        let vtype: TokenStream = vinner.parse()?;
                        vs.push((vname, vtype));
                        if !inner.is_empty() && inner.peek(Token![,]) {
                            inner.parse::<Token![,]>()?;
                        }
                    }
                    vs
                };
                MetaParamKind::DynDispatch {
                    c_name_suffix,
                    variants,
                }
            }
            other => {
                return Err(syn::Error::new(
                    kind_ident.span(),
                    format!("unknown param kind `{other}`"),
                ));
            }
        };
        parse_comma(input)?;

        // rust_type is optional (not present for some kinds)
        let rust_type = if !input.is_empty() {
            expect_key(input, "rust_type")?;
            let rt = parse_parenthesized_tokens(input)?;
            parse_comma(input)?;
            Some(rt)
        } else {
            None
        };

        Ok(MetaParam {
            name,
            kind,
            rust_type,
        })
    }
}

impl syn::parse::Parse for MetaReturn {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kind: Ident = input.parse()?;
        match kind.to_string().as_str() {
            "void" => Ok(MetaReturn::Void),
            "value" => {
                let content;
                syn::parenthesized!(content in input);
                let vk = content.parse::<MetaValueKind>()?;
                Ok(MetaReturn::Value(vk))
            }
            "result" => {
                let content;
                syn::parenthesized!(content in input);

                expect_key(&content, "ok")?;
                let ok_kind: Ident = content.parse()?;
                let ok = if ok_kind == "void" {
                    None
                } else if ok_kind == "some" {
                    let inner;
                    syn::parenthesized!(inner in content);
                    Some(inner.parse::<MetaValueKind>()?)
                } else {
                    return Err(syn::Error::new(ok_kind.span(), "expected `void` or `some`"));
                };
                content.parse::<Token![,]>()?;

                expect_key(&content, "err_bridge_type")?;
                let err_bridge_type = parse_parenthesized_tokens(&content)?;
                content.parse::<Token![,]>()?;

                expect_key(&content, "err_ident")?;
                let err_ident = parse_string(&content)?;
                parse_comma(&content)?;

                Ok(MetaReturn::Result {
                    ok,
                    err_bridge_type,
                    err_ident,
                })
            }
            other => Err(syn::Error::new(
                kind.span(),
                format!("unknown return kind `{other}`"),
            )),
        }
    }
}

impl syn::parse::Parse for MetaValueKind {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kind: Ident = input.parse()?;
        match kind.to_string().as_str() {
            "regular" => {
                parse_comma(input)?;
                expect_key(input, "bridge_type")?;
                let bridge_type = parse_parenthesized_tokens(input)?;
                parse_comma(input)?;
                Ok(MetaValueKind::Regular { bridge_type })
            }
            other => Err(syn::Error::new(
                kind.span(),
                format!("unknown value kind `{other}`"),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Error metadata parsing
// ---------------------------------------------------------------------------

impl syn::parse::Parse for MetaError {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![@]>()?;
        let tag: Ident = input.parse()?;
        if tag != "error" {
            return Err(syn::Error::new(tag.span(), "expected `error`"));
        }
        parse_comma(input)?;

        expect_key(input, "name")?;
        let name: Ident = input.parse()?;
        parse_comma(input)?;

        expect_key(input, "path")?;
        let path = parse_parenthesized_tokens(input)?;
        parse_comma(input)?;

        expect_key(input, "prefix")?;
        let prefix = parse_string(input)?;
        parse_comma(input)?;

        expect_key(input, "variants")?;
        let variants = {
            let content;
            syn::bracketed!(content in input);
            let mut vs = Vec::new();
            while !content.is_empty() {
                let inner;
                syn::braced!(inner in content);
                expect_key(&inner, "name")?;
                let vname: Ident = inner.parse()?;
                parse_comma(&inner)?;
                expect_key(&inner, "code")?;
                let code: syn::LitInt = inner.parse()?;
                let code = code.base10_parse::<u64>()?;
                parse_comma(&inner)?;
                expect_key(&inner, "message")?;
                let message = parse_string(&inner)?;
                parse_comma(&inner)?;
                vs.push(MetaErrorVariant {
                    name: vname,
                    code,
                    message,
                });
                if !content.is_empty() && content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
            vs
        };
        parse_comma(input)?;

        Ok(MetaError {
            name,
            path,
            prefix,
            variants,
        })
    }
}

// ---------------------------------------------------------------------------
// Implementable metadata parsing
// ---------------------------------------------------------------------------

impl syn::parse::Parse for MetaImplementable {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![@]>()?;
        let tag: Ident = input.parse()?;
        if tag != "implementable" {
            return Err(syn::Error::new(tag.span(), "expected `implementable`"));
        }
        parse_comma(input)?;

        expect_key(input, "trait_name")?;
        let trait_name: Ident = input.parse()?;
        parse_comma(input)?;

        expect_key(input, "trait_path")?;
        let trait_path = parse_parenthesized_tokens(input)?;
        parse_comma(input)?;

        expect_key(input, "prefix")?;
        let prefix = parse_string(input)?;
        parse_comma(input)?;

        expect_key(input, "vtable_struct")?;
        let vtable_struct_name = parse_parenthesized_tokens(input)?;
        parse_comma(input)?;

        expect_key(input, "wrapper_name")?;
        let wrapper_name = parse_parenthesized_tokens(input)?;
        parse_comma(input)?;

        expect_key(input, "vtable_fields")?;
        let vtable_fields = {
            let content;
            syn::bracketed!(content in input);
            let mut fs = Vec::new();
            while !content.is_empty() {
                let inner;
                syn::braced!(inner in content);
                expect_key(&inner, "name")?;
                let fname: Ident = inner.parse()?;
                parse_comma(&inner)?;
                expect_key(&inner, "field_type")?;
                let ftype = parse_parenthesized_tokens(&inner)?;
                parse_comma(&inner)?;
                fs.push(MetaVtableField {
                    name: fname,
                    field_type: ftype,
                });
                if !content.is_empty() && content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
            fs
        };
        parse_comma(input)?;

        expect_key(input, "vtable_methods")?;
        let vtable_methods = {
            let content;
            syn::bracketed!(content in input);
            let mut ms = Vec::new();
            while !content.is_empty() {
                let inner;
                syn::braced!(inner in content);
                expect_key(&inner, "name")?;
                let mname: Ident = inner.parse()?;
                parse_comma(&inner)?;
                expect_key(&inner, "params")?;
                let params = {
                    let pinner;
                    syn::bracketed!(pinner in inner);
                    let mut ps = Vec::new();
                    while !pinner.is_empty() {
                        ps.push(parse_vtable_param(&pinner)?);
                        if !pinner.is_empty() && pinner.peek(Token![,]) {
                            pinner.parse::<Token![,]>()?;
                        }
                    }
                    ps
                };
                parse_comma(&inner)?;
                expect_key(&inner, "ret")?;
                let ret = parse_vtable_ret(&inner)?;
                parse_comma(&inner)?;
                ms.push(MetaVtableMethod {
                    name: mname,
                    params,
                    ret,
                });
                if !content.is_empty() && content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
            ms
        };
        parse_comma(input)?;

        Ok(MetaImplementable {
            trait_name,
            trait_path,
            prefix,
            vtable_struct_name,
            wrapper_name,
            vtable_fields,
            vtable_methods,
        })
    }
}

fn parse_vtable_param(input: ParseStream) -> syn::Result<MetaVtableParam> {
    let content;
    syn::braced!(content in input);
    expect_key(&content, "name")?;
    let name: Ident = content.parse()?;
    parse_comma(&content)?;
    expect_key(&content, "bridge_type")?;
    let bridge_type = parse_parenthesized_tokens(&content)?;
    parse_comma(&content)?;
    expect_key(&content, "rust_type")?;
    let rust_type = parse_parenthesized_tokens(&content)?;
    parse_comma(&content)?;
    Ok(MetaVtableParam {
        name,
        bridge_type,
        rust_type,
    })
}

fn parse_vtable_ret(input: ParseStream) -> syn::Result<MetaVtableRet> {
    let kind: Ident = input.parse()?;
    match kind.to_string().as_str() {
        "void" => Ok(MetaVtableRet::Void),
        "value" => {
            let content;
            syn::parenthesized!(content in input);
            expect_key(&content, "bridge_type")?;
            let bridge_type = parse_parenthesized_tokens(&content)?;
            parse_comma(&content)?;
            expect_key(&content, "rust_type")?;
            let rust_type = parse_parenthesized_tokens(&content)?;
            parse_comma(&content)?;
            Ok(MetaVtableRet::Value {
                bridge_type,
                rust_type,
            })
        }
        other => Err(syn::Error::new(
            kind.span(),
            format!("unknown vtable ret type `{other}`"),
        )),
    }
}

// ---------------------------------------------------------------------------
// Trait impl metadata parsing
// ---------------------------------------------------------------------------

impl syn::parse::Parse for MetaTraitImpl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![@]>()?;
        let tag: Ident = input.parse()?;
        if tag != "trait_impl" {
            return Err(syn::Error::new(tag.span(), "expected `trait_impl`"));
        }
        parse_comma(input)?;

        expect_key(input, "trait_name")?;
        let trait_name: Ident = input.parse()?;
        parse_comma(input)?;

        expect_key(input, "struct_name")?;
        let struct_name: Ident = input.parse()?;
        parse_comma(input)?;

        expect_key(input, "struct_path")?;
        let struct_path = parse_parenthesized_tokens(input)?;
        parse_comma(input)?;

        expect_key(input, "trait_path")?;
        let trait_path = parse_parenthesized_tokens(input)?;
        parse_comma(input)?;

        expect_key(input, "prefix")?;
        let prefix = parse_string(input)?;
        parse_comma(input)?;

        expect_key(input, "lifetimes")?;
        let lifetimes = {
            let content;
            syn::parenthesized!(content in input);
            let mut lts = Vec::new();
            while !content.is_empty() {
                lts.push(content.parse::<Ident>()?);
                if !content.is_empty() {
                    content.parse::<Token![,]>()?;
                }
            }
            lts
        };
        parse_comma(input)?;

        expect_key(input, "trait_lifetime_args")?;
        let trait_lifetime_args = {
            let content;
            syn::bracketed!(content in input);
            let mut args = Vec::new();
            while !content.is_empty() {
                args.push(parse_string(&content)?);
                if !content.is_empty() && content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
            args
        };
        parse_comma(input)?;

        expect_key(input, "methods")?;
        let methods = {
            let content;
            syn::bracketed!(content in input);
            let mut ms = Vec::new();
            while !content.is_empty() {
                let inner;
                syn::braced!(inner in content);
                expect_key(&inner, "name")?;
                let mname: Ident = inner.parse()?;
                parse_comma(&inner)?;
                expect_key(&inner, "params")?;
                let params = {
                    let pinner;
                    syn::bracketed!(pinner in inner);
                    let mut ps = Vec::new();
                    while !pinner.is_empty() {
                        ps.push(parse_vtable_param(&pinner)?);
                        if !pinner.is_empty() && pinner.peek(Token![,]) {
                            pinner.parse::<Token![,]>()?;
                        }
                    }
                    ps
                };
                parse_comma(&inner)?;
                expect_key(&inner, "ret")?;
                let ret = parse_vtable_ret(&inner)?;
                parse_comma(&inner)?;
                ms.push(MetaVtableMethod {
                    name: mname,
                    params,
                    ret,
                });
                if !content.is_empty() && content.peek(Token![,]) {
                    content.parse::<Token![,]>()?;
                }
            }
            ms
        };
        parse_comma(input)?;

        Ok(MetaTraitImpl {
            trait_name,
            struct_name,
            struct_path,
            trait_path,
            prefix,
            lifetimes,
            trait_lifetime_args,
            methods,
        })
    }
}

