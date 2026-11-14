//! C ABI signatures and type resolution for bridge methods.

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use crate::c_abi::{CParam, c_signature};
use crate::meta::{MetaMethod, MetaReceiver};

/// Complete C extern function signature for a method.
///
/// Contains all the information needed to emit an `unsafe extern "C" { fn ... }`
/// declaration. The parameters include handle, regular params, and out-param
/// (for Result returns) in the order they appear.
pub(super) struct CExternSignature {
    /// Fully qualified extern function name (e.g. "mylib_calculator_add").
    _fn_name: String,
    /// All parameters in declaration order.
    pub(super) params: Vec<CParam>,
    /// Return type tokens (empty for void).
    pub(super) ret: TokenStream2,
}

/// Compute the full C extern signature for a method: the receiver handle
/// (if any) followed by [`c_signature`].
pub(super) fn c_signature_for_method(
    method: &MetaMethod,
    prefix: &str,
    handle_types: &HashSet<String>,
    value_types: &HashSet<String>,
    lib_crate: &TokenStream2,
) -> CExternSignature {
    let fn_name = format!("{}_{}", prefix, method.ffi_name());
    let mut params = Vec::new();
    // All handles are heap-allocated and passed as *mut c_void.
    if method.receiver != MetaReceiver::None {
        params.push(CParam {
            name: format_ident!("handle"),
            c_type: quote! { *mut core::ffi::c_void },
        });
    }
    let local_token = crate::ffi_traits::token_of(lib_crate);
    let sig = c_signature(method, handle_types, value_types, &local_token);
    params.extend(sig.params);
    CExternSignature {
        _fn_name: fn_name,
        params,
        ret: sig.ret,
    }
}
