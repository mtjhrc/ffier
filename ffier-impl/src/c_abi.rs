//! The C ABI parameter and return list of a method, shared by the bridge's
//! exported functions and the vtable caller's function pointers.

use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::meta::{MetaMethod, MetaParamKind, MetaTypePair};
use crate::return_shape::{ResultConv, ReturnShape, classify_return};

/// A single parameter of a C signature.
pub(crate) struct CParam {
    pub(crate) name: syn::Ident,
    pub(crate) c_type: TokenStream,
}

/// A method's C parameters (excluding the leading receiver or user-data
/// pointer, which the caller adds) and return type.
pub(crate) struct CSignature {
    /// Parameters, then return out-params, in declaration order.
    pub(crate) params: Vec<CParam>,
    /// Return type tokens including `->`, empty for void.
    pub(crate) ret: TokenStream,
}

/// The token whose conversion impls cover `tp`: the foreign library's for a
/// foreign type, `local_token` otherwise.
pub(crate) fn token_for(tp: &MetaTypePair, local_token: &TokenStream) -> TokenStream {
    match &tp.foreign_crate {
        Some(foreign_crate) => crate::ffi_traits::token_of(foreign_crate),
        None => local_token.clone(),
    }
}

/// `<T as FfiType<Tok>>::CRepr` for a bridge type.
fn c_repr(ty: &TokenStream, tok: &TokenStream) -> TokenStream {
    quote! { <#ty as ffier::FfiType<#tok>>::CRepr }
}

/// Compute a method's C parameters and return.
///
/// `local_token` names this library's token as seen from where the
/// signature is emitted. The return follows [`classify_return`].
pub(crate) fn c_signature(
    method: &MetaMethod,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    local_token: &TokenStream,
) -> CSignature {
    let mut params = Vec::new();
    let mut push = |name: syn::Ident, c_type: TokenStream| params.push(CParam { name, c_type });

    for p in &method.params {
        match &p.kind {
            MetaParamKind::Regular(tp) => {
                push(
                    p.name.clone(),
                    c_repr(&tp.bridge_type, &token_for(tp, local_token)),
                );
            }
            MetaParamKind::ImplTrait { .. } => {
                push(p.name.clone(), quote! { *mut core::ffi::c_void })
            }
            MetaParamKind::StrSlice => {
                push(p.name.clone(), quote! { *const ffier::FfierBytes });
                push(format_ident!("{}_len", p.name), quote! { usize });
            }
            MetaParamKind::PrimitiveSlice(tp) => {
                let elem = c_repr(&tp.bridge_type, &token_for(tp, local_token));
                push(p.name.clone(), quote! { *const #elem });
                push(format_ident!("{}_len", p.name), quote! { usize });
            }
        }
    }

    let out_param = |name: &str| format_ident!("{name}");
    let err_out = quote! { *mut *mut core::ffi::c_void };
    let ret = match classify_return(&method.ret, &method.rust_ret, handle_types, value_types) {
        ReturnShape::Void => quote! {},
        ReturnShape::OptionalValue { inner, .. } => {
            let repr = c_repr(&inner, local_token);
            push(out_param("result"), quote! { *mut #repr });
            quote! { -> bool }
        }
        ReturnShape::Value { tp, .. } => {
            let repr = c_repr(&tp.bridge_type, &token_for(tp, local_token));
            quote! { -> #repr }
        }
        ReturnShape::Result { ok, conv, .. } => match conv {
            ResultConv::OptionalValueOutParam { inner } => {
                let repr = c_repr(&inner, local_token);
                push(out_param("result_is_some"), quote! { *mut bool });
                push(out_param("result"), quote! { *mut #repr });
                push(out_param("err_out"), err_out);
                quote! { -> ffier::FfierResult }
            }
            // The handle is the return value, NULL on error.
            ResultConv::OwnedHandleOrNull => {
                push(out_param("err_out"), err_out);
                quote! { -> *mut core::ffi::c_void }
            }
            ResultConv::OutParam => {
                // A returned Self that replaces the receiver has no ok here:
                // it is written back through the handle param.
                if let Some(tp) = ok {
                    let repr = c_repr(&tp.bridge_type, &token_for(tp, local_token));
                    push(out_param("result"), quote! { *mut #repr });
                }
                push(out_param("err_out"), err_out);
                quote! { -> ffier::FfierResult }
            }
        },
    };

    CSignature { params, ret }
}
