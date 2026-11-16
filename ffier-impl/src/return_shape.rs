//! The one classification of a method return for the C ABI.
//!
//! The C signature, the bridge's return conversion, the schema's
//! `CResultConvention`, and the vtable caller all derive their behavior from
//! [`classify_return`], so they cannot disagree about how a return crosses
//! the boundary.

use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::quote;

use crate::meta::{
    MetaReturn, MetaTypePair, is_result_ok_handle, option_inner_tokens, option_value_struct_inner,
    type_last_name,
};

/// How a method's return crosses the C ABI.
pub(crate) enum ReturnShape<'a> {
    Void,
    /// `Option<ValueStruct>`: the presence flag is the C return value; the
    /// value goes through a `result` out-param. `inner` is the bridge-side
    /// value type without the `Option`.
    OptionalValue {
        tp: &'a MetaTypePair,
        inner: TokenStream,
    },
    /// A value returned as its C representation.
    Value {
        tp: &'a MetaTypePair,
    },
    /// `Result<T, E>`.
    Result {
        ok: Option<&'a MetaTypePair>,
        err_ident: &'a str,
        conv: ResultConv,
    },
}

/// C calling convention of a `Result` return.
pub(crate) enum ResultConv {
    /// `FfierResult` status; the ok value (if any) goes through `result`.
    OutParam,
    /// The owned ok handle is returned directly, null on error.
    OwnedHandleOrNull,
    /// `Result<Option<ValueStruct>, E>`: presence through `result_is_some`,
    /// value through `result`. `inner` is the bridge-side value type.
    OptionalValueOutParam { inner: TokenStream },
}

impl ResultConv {
    pub(crate) fn c_convention(&self) -> ffier_schema::CResultConvention {
        match self {
            ResultConv::OutParam => ffier_schema::CResultConvention::OutParam,
            ResultConv::OwnedHandleOrNull => ffier_schema::CResultConvention::HandleOrNull,
            ResultConv::OptionalValueOutParam { .. } => {
                ffier_schema::CResultConvention::OptionalValueOutParam
            }
        }
    }

    /// The ok handle is the C return value.
    pub(crate) fn is_handle_or_null(&self) -> bool {
        matches!(self, ResultConv::OwnedHandleOrNull)
    }
}

/// Classify a method return.
///
/// A returned `Self` that replaces the receiver needs no special case here:
/// its metadata already carries `Void` or an `ok`-less `Result`.
pub(crate) fn classify_return<'a>(
    ret: &'a MetaReturn,
    rust_ret: &TokenStream,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
) -> ReturnShape<'a> {
    // Classification reads the Rust type; emitted code uses the
    // definition-site-resolved bridge type.
    let optional_value_inner = |tp: &MetaTypePair| {
        option_value_struct_inner(&tp.rust_type, value_types)
            .and_then(|_| option_inner_tokens(&tp.bridge_type))
    };
    match ret {
        MetaReturn::Void => ReturnShape::Void,
        MetaReturn::Value(tp) => match optional_value_inner(tp) {
            Some(inner) => ReturnShape::OptionalValue { tp, inner },
            None => ReturnShape::Value { tp },
        },
        MetaReturn::Result { ok, err_ident } => {
            let conv = match ok {
                None => ResultConv::OutParam,
                Some(tp) => {
                    if let Some(inner) = optional_value_inner(tp) {
                        ResultConv::OptionalValueOutParam { inner }
                    } else if tp.foreign_crate.is_some() {
                        // A foreign ok type is always an owned handle.
                        ResultConv::OwnedHandleOrNull
                    } else if is_result_ok_handle(rust_ret, handle_types) {
                        ResultConv::OwnedHandleOrNull
                    } else {
                        ResultConv::OutParam
                    }
                }
            };
            ReturnShape::Result {
                ok: ok.as_ref(),
                err_ident,
                conv,
            }
        }
    }
}

/// Reject a method that returns a borrowed handle: `&H`, `&mut H` or
/// `Result<&H, E>` of a local or foreign handle, including `&Self`. A
/// receiver-replacing `&mut self -> &mut Self` is not a returned value and
/// stays allowed.
///
/// Neither C nor a generated client can keep such a handle tied to its
/// owner's lifetime, so these returns are not supported. `owner` names the
/// type or trait in the error.
pub(crate) fn reject_borrowed_handle_return(
    owner: Option<&dyn std::fmt::Display>,
    method: &crate::meta::MetaMethod,
    handle_types: &HashSet<String>,
) -> Option<TokenStream> {
    let tp = match &method.ret {
        MetaReturn::Value(tp) | MetaReturn::Result { ok: Some(tp), .. } => tp,
        _ => return None,
    };
    let ty = syn::parse2::<syn::Type>(tp.rust_type.clone()).ok()?;
    if !matches!(ty, syn::Type::Reference(_))
        || !(tp.foreign_crate.is_some() || is_borrowed_handle(&ty, handle_types))
        || is_std_reference(&ty)
    {
        return None;
    }
    let name = match owner {
        Some(owner) => format!("{owner}::{}", method.name),
        None => method.name.to_string(),
    };
    let shown = quote!(#ty).to_string().replace("& ", "&");
    let msg = format!(
        "ffier: `{name}` returns the borrowed handle `{shown}`, which is not supported; \
         return an owned handle or a value instead"
    );
    Some(syn::Error::new_spanned(&tp.rust_type, msg).to_compile_error())
}

/// The type-level form of [`reject_borrowed_handle_return`], which a type
/// alias cannot hide: a compile-time assertion that the method's return is
/// not a borrowed handle, gated by the method's cfg. `local_token` names
/// this library's token where the assertion is emitted.
pub(crate) fn assert_no_borrowed_handle_return(
    owner: Option<&dyn std::fmt::Display>,
    method: &crate::meta::MetaMethod,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    local_token: &TokenStream,
) -> Option<TokenStream> {
    let tp = match classify_return(&method.ret, &method.rust_ret, handle_types, value_types) {
        ReturnShape::Value { tp } => tp,
        ReturnShape::Result {
            ok: Some(tp),
            conv: ResultConv::OutParam | ResultConv::OwnedHandleOrNull,
            ..
        } => tp,
        _ => return None,
    };
    let bt = &tp.bridge_type;
    let tok = crate::c_abi::token_for(tp, local_token);
    let cfg = method.cfg_attr();
    let name = match owner {
        Some(owner) => format!("{owner}::{}", method.name),
        None => method.name.to_string(),
    };
    let msg = format!(
        "ffier: `{name}` returns a borrowed handle, which is not supported; \
         return an owned handle or a value instead"
    );
    Some(quote! {
        #cfg
        const _: () = assert!(!<#bt as ffier::FfiType<#tok>>::IS_BORROWED_HANDLE, #msg);
    })
}

/// A reference into `std`, `core` or `alloc` (`&std::path::Path`), which no
/// handle name can shadow.
fn is_std_reference(ty: &syn::Type) -> bool {
    let syn::Type::Reference(r) = ty else {
        return false;
    };
    let syn::Type::Path(p) = &*r.elem else {
        return false;
    };
    p.path
        .segments
        .first()
        .is_some_and(|s| matches!(s.ident.to_string().as_str(), "std" | "core" | "alloc"))
        && p.path.segments.len() > 1
}

/// Check if a type is `&T` or `&mut T` where `T` is a handle type.
pub(crate) fn is_borrowed_handle(ty: &syn::Type, handle_types: &HashSet<String>) -> bool {
    if let syn::Type::Reference(ref_ty) = ty {
        type_last_name(&ref_ty.elem)
            .is_some_and(|name| name == "Self" || handle_types.contains(&name))
    } else {
        false
    }
}

/// The bridge type of a borrowed handle with its reference stripped
/// (`&Gadget` → `Gadget`).
pub(crate) fn borrowed_handle_target(tp: &MetaTypePair) -> TokenStream {
    match syn::parse2::<syn::Type>(tp.bridge_type.clone()) {
        Ok(syn::Type::Reference(ref_ty)) => {
            let elem = &ref_ty.elem;
            quote! { #elem }
        }
        _ => tp.bridge_type.clone(),
    }
}
