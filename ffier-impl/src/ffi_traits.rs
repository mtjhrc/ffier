//! Emitting impls of and paths to the shared `ffier` conversion traits.
//!
//! The traits live in `ffier-rt` and take a token type: each library crate
//! defines `__FfierLocalToken` (see `library_definition!`) and implements
//! the traits with it. Code expanded inside the library crate names the
//! token bare; the bridge names it through a crate path.

use proc_macro2::TokenStream;
use quote::quote;

/// The token defined by `library_definition!` in the crate the emitted code
/// expands in.
pub(crate) fn local_token() -> TokenStream {
    quote! { __FfierLocalToken }
}

/// The token of the library crate at `crate_path` (`$crate`, `::other_lib`).
pub(crate) fn token_of(crate_path: &TokenStream) -> TokenStream {
    quote! { #crate_path::__FfierLocalToken }
}

/// `FfiType` + `IntoC` + `FromC` impls for `ty` under the local token.
///
/// `into_c` is the body of `fn into_c(self)`; `from_c` is the body of
/// `unsafe fn from_c(repr)`.
pub(crate) fn local_conversions(
    impl_generics: &TokenStream,
    ty: &TokenStream,
    repr: &TokenStream,
    into_c: TokenStream,
    from_c: TokenStream,
) -> TokenStream {
    let tok = local_token();
    quote! {
        impl #impl_generics ffier::FfiType<#tok> for #ty {
            type CRepr = #repr;
        }
        impl #impl_generics ffier::IntoC<#tok> for #ty {
            fn into_c(self) -> #repr { #into_c }
        }
        impl #impl_generics ffier::FromC<#tok> for #ty {
            unsafe fn from_c(repr: #repr) -> Self { #from_c }
        }
    }
}

/// The impls of a native handle type: the value is moved into a new
/// ffier-allocated handle and consumed back out of it.
pub(crate) fn native_handle_impls(
    impl_generics: &TokenStream,
    ty: &TokenStream,
    c_handle_name: &TokenStream,
    type_tag: &TokenStream,
) -> TokenStream {
    let tok = local_token();
    quote! {
        impl #impl_generics ffier::FfiHandle<#tok> for #ty {
            const C_HANDLE_NAME: &'static str = #c_handle_name;
            const TYPE_TAG: u32 = #type_tag;
        }
        impl #impl_generics ffier::NativeHandle<#tok> for #ty {}
        impl #impl_generics ffier::FfiType<#tok> for #ty {
            type CRepr = *mut core::ffi::c_void;
            const IS_HANDLE: bool = true;
        }
        impl #impl_generics ffier::IntoC<#tok> for #ty {
            fn into_c(self) -> *mut core::ffi::c_void {
                ffier::ffier_handle_new(#type_tag, self)
            }
        }
        impl #impl_generics ffier::FromC<#tok> for #ty {
            unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {
                unsafe { ffier::ffier_handle_consume::<Self>(repr) }
            }
        }
    }
}
