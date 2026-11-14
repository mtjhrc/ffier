//! Native error construction, payload access, and result-message bridges.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use crate::meta::MetaError;

/// Construct native fieldless errors in the library's allocator.
pub(super) fn generate_error_factory(
    prefix: &str,
    errors: &[TokenStream2],
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    let factory_fn = format_ident!("{prefix}_error_new");
    let mut dispatch_arms = Vec::new();
    for item in errors {
        let meta: MetaError = syn::parse2(item.clone()).expect("validated error metadata");
        let type_tag = meta.type_tag;
        let path = &meta.path;
        dispatch_arms.push(quote! {
            #type_tag => <#path as ffier::FfiError>::from_code(code)
                .map(<#path as #lib_crate::FfiType>::into_c)
                .unwrap_or(core::ptr::null_mut()),
        });
    }
    quote! {
        /// Create an owned fieldless error. Returns null for success,
        /// unknown error types/codes, and variants with payloads.
        /// Destroy the handle exactly once or transfer it to a callback caller.
        #[unsafe(no_mangle)]
        pub extern "C" fn #factory_fn(r: ffier::FfierResult) -> *mut core::ffi::c_void {
            let code = ffier::ffier_result_code(r);
            match ffier::ffier_result_type_tag(r) {
                #(#dispatch_arms)*
                _ => core::ptr::null_mut(),
            }
        }
    }
}

/// Generate `{prefix}_error_payload(handle, index, out_buf, buf_size)` — shallow-
/// copies the indexed field's CRepr into caller-provided storage, *borrowing* the
/// data from the handle (handle stays alive, caller must not outlive it).
///
/// For `Box<str>` payloads this writes a `FfierBytes { data, len }` pointing
/// into the handle's owned string. Invalid or opaque field access panics.
pub(super) fn generate_error_getters(
    prefix: &str,
    errors: &[TokenStream2],
    _lib_crate: &TokenStream2,
) -> TokenStream2 {
    let fn_pfx = format!("{prefix}_");
    let payload_fn = format_ident!("{fn_pfx}error_payload");

    let mut dispatch_arms = Vec::new();

    for item in errors {
        let Ok(meta) = syn::parse2::<MetaError>(item.clone()) else {
            continue;
        };
        let type_tag = meta.type_tag;
        let path = &meta.path;

        dispatch_arms.push(quote! {
            #type_tag => {
                let err: &#path = unsafe { ffier::ffier_handle_borrow(handle) };
                unsafe { ffier::FfiError::payload(err, index, out_buf, buf_size) };
            }
        });
    }

    quote! {
        /// Shallow-copy the indexed error field into caller-provided
        /// storage. The written CRepr borrows from the handle — the
        /// handle must stay alive while the caller uses the data.
        ///
        /// Invalid or opaque field access and undersized buffers panic. Panics
        /// abort the process at the C ABI boundary; they do not unwind into C.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #payload_fn(
            handle: *const core::ffi::c_void,
            index: usize,
            out_buf: *mut core::ffi::c_void,
            buf_size: usize,
        ) {
            if handle.is_null() { return; }
            let type_tag = unsafe { ffier::handle_type_tag(handle) };
            match type_tag {
                #(#dispatch_arms)*
                _ => {}
            }
        }
    }
}

/// Generate result name lookups and the allocation-free result message lookup.
///
/// These decode packed `FfierResult` values (type_tag + code) into static
/// names or messages. Error handle dispatch (destroy, message, code, result)
/// is handled by the Error trait's self-dispatch infrastructure.
pub(super) fn generate_strerror_bridge(
    prefix: &str,
    errors: &[TokenStream2],
    lib_crate: &TokenStream2,
) -> TokenStream2 {
    let fn_pfx = format!("{prefix}_");

    let result_name_cstr_fn = format_ident!("{fn_pfx}result_name_cstr");
    let result_name_fn = format_ident!("{fn_pfx}result_name");
    let result_message_fn = format_ident!("{fn_pfx}result_message");

    let mut result_name_dispatch_arms = Vec::new();
    let mut result_message_dispatch_arms = Vec::new();
    for item in errors {
        if let Ok(meta) = syn::parse2::<MetaError>(item.clone()) {
            let type_tag = meta.type_tag;
            let path = &meta.path;
            let names = meta.variants.iter().map(|variant| {
                let code = variant.code;
                let name = format!("{}\0", variant.name);
                let name = proc_macro2::Literal::byte_string(name.as_bytes());
                quote! { #code => #name.as_ptr().cast() }
            });
            result_name_dispatch_arms.push(quote! {
                #type_tag => match ffier::ffier_result_code(r) {
                    #(#names,)*
                    _ => b"unknown error\0".as_ptr().cast(),
                }
            });
            result_message_dispatch_arms.push(quote! {
                #type_tag => {
                    let code = ffier::ffier_result_code(r);
                    let message = <#path as #lib_crate::FfiError>::static_message(code).to_bytes();
                    ffier::FfierBytes { data: message.as_ptr(), len: message.len() }
                }
            });
        }
    }

    let unknown_msg_bytes = proc_macro2::Literal::byte_string(b"unknown error\0");

    quote! {
        /// Variant name as a null-terminated C string.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #result_name_cstr_fn(
            r: ffier::FfierResult,
        ) -> *const core::ffi::c_char {
            if r == 0 { return b"success\0".as_ptr() as *const core::ffi::c_char; }
            let type_tag = ffier::ffier_result_type_tag(r);
            match type_tag {
                #(#result_name_dispatch_arms)*
                _ => unsafe {
                    core::ffi::CStr::from_bytes_with_nul_unchecked(#unknown_msg_bytes).as_ptr()
                },
            }
        }

        /// Variant name as `FtStr` (length-prefixed, no strlen needed).
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #result_name_fn(
            r: ffier::FfierResult,
        ) -> ffier::FfierBytes {
            let ptr = unsafe { #result_name_cstr_fn(r) };
            let len = unsafe { core::ffi::CStr::from_ptr(ptr) }.to_bytes().len();
            ffier::FfierBytes { data: ptr as *const u8, len }
        }

        /// Static message, falling back to the raw variant name when omitted.
        /// The returned UTF-8 string needs no allocation or error handle and
        /// remains valid while the library is loaded. Payload values and native
        /// Display are not evaluated.
        #[unsafe(no_mangle)]
        pub extern "C" fn #result_message_fn(r: ffier::FfierResult) -> ffier::FfierBytes {
            if r == 0 { return unsafe { ffier::FfierBytes::from_str("success") }; }
            match ffier::ffier_result_type_tag(r) {
                #(#result_message_dispatch_arms)*
                _ => unsafe { ffier::FfierBytes::from_str("unknown error") },
            }
        }
    }
}

pub(super) fn generate_error_bridge(_meta: MetaError, _lib_crate: &TokenStream2) -> TokenStream2 {
    // No per-type bridge functions needed — strerror is emitted at batch level.
    quote! {}
}
