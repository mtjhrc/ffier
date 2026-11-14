//! Library-level helper for freeing strings.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

/// Generate `{prefix}_str_free(FfierBytes s)` that reconstitutes and drops a
/// `Box<str>` previously leaked via `Box::leak` in a `Box<str>` return.
pub(super) fn generate_str_free(prefix: &str) -> TokenStream2 {
    let fn_pfx = format!("{prefix}_");
    let str_free_fn = format_ident!("{fn_pfx}str_free");

    quote! {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #str_free_fn(s: ffier::FfierBytes) {
            if !s.data.is_null() {
                unsafe {
                    let slice = core::slice::from_raw_parts_mut(s.data as *mut u8, s.len);
                    drop(Box::from_raw(core::str::from_utf8_unchecked_mut(slice)));
                }
            }
        }
    }
}
