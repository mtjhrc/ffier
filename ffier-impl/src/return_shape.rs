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
    MetaReturn, MetaTypePair, extract_result_ok_type, is_result_ok_handle, option_inner_tokens,
    option_value_struct_inner, type_last_name,
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
        /// `&Handle`: returned as a borrowed handle shell.
        borrowed_handle: bool,
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
    /// A borrowed `&Handle` ok is returned as a borrowed shell, null on error.
    BorrowedHandleOrNull,
    /// `Result<Option<ValueStruct>, E>`: presence through `result_is_some`,
    /// value through `result`. `inner` is the bridge-side value type.
    OptionalValueOutParam { inner: TokenStream },
}

impl ResultConv {
    pub(crate) fn c_convention(&self) -> ffier_schema::CResultConvention {
        match self {
            ResultConv::OutParam => ffier_schema::CResultConvention::OutParam,
            ResultConv::OwnedHandleOrNull | ResultConv::BorrowedHandleOrNull => {
                ffier_schema::CResultConvention::HandleOrNull
            }
            ResultConv::OptionalValueOutParam { .. } => {
                ffier_schema::CResultConvention::OptionalValueOutParam
            }
        }
    }

    /// The ok handle is the C return value.
    pub(crate) fn is_handle_or_null(&self) -> bool {
        matches!(
            self,
            ResultConv::OwnedHandleOrNull | ResultConv::BorrowedHandleOrNull
        )
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
            None => ReturnShape::Value {
                tp,
                // Foreign types are owned handles of another library.
                borrowed_handle: tp.foreign_crate.is_none()
                    && syn::parse2::<syn::Type>(rust_ret.clone())
                        .is_ok_and(|ty| is_borrowed_handle(&ty, handle_types)),
            },
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
                    } else if syn::parse2::<syn::Type>(extract_result_ok_type(rust_ret))
                        .is_ok_and(|ty| is_borrowed_handle(&ty, handle_types))
                    {
                        ResultConv::BorrowedHandleOrNull
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

/// Check if a type is `&T` or `&mut T` where `T` is a handle type.
fn is_borrowed_handle(ty: &syn::Type, handle_types: &HashSet<String>) -> bool {
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
