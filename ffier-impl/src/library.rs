//! Library registration parsing and metadata macro chain generation.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{LitStr, Token, parse::Parse, parse_macro_input};

use crate::cfg;
use crate::meta::camel_to_snake;

pub(crate) fn library_definition(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as LibraryInput);
    let prefix_lit = &parsed.prefix;
    let prefix_str = parsed.prefix.value();
    let library_tag = parsed.library_tag;
    let primitives_prefix_lit = parsed.primitives_prefix.as_ref().unwrap_or(&parsed.prefix);

    // For each entry, compute:
    // 1. shim_macros: shim macros that inject the tag into the metadata macro call
    // 2. reexport_invocations: `alias_path!(@on_library_export, ...)` calls
    // 3. shim_names: bare ident of each shim macro (entry macro adds $crate:: or not)
    let mut shim_macros: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut reexport_invocations: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut shim_names: Vec<syn::Ident> = Vec::new();

    // Collect names of external traits (from TaggedTrait entries) so that
    // TraitImpl entries can use the internal alias instead of bare names.
    let external_trait_names: std::collections::HashSet<String> = parsed
        .entries
        .iter()
        .filter_map(|e| {
            if let LibraryEntry::TaggedTrait(path, _) = &e.entry {
                let is_external = path.segments.len() > 1
                    && path.segments.first().is_none_or(|seg| seg.ident != "crate");
                if is_external {
                    return Some(path_last_ident(path).to_string());
                }
            }
            None
        })
        .collect();

    // Collect handle type names — used to determine GLib vs FfierResult
    // convention for Result-returning vtable methods. Includes both
    // plain Tagged types (Widget, Gadget) and VtableFoo wrappers from
    // TaggedTrait entries to match the bridge's handle_types set.
    let handle_type_idents: Vec<syn::Ident> = parsed
        .entries
        .iter()
        .filter_map(|e| match &e.entry {
            LibraryEntry::Tagged(path, _) => Some(path_last_ident(path).clone()),
            LibraryEntry::TaggedTrait(path, _) => {
                let trait_name = path_last_ident(path);
                Some(format_ident!("Vtable{trait_name}"))
            }
            _ => None,
        })
        .collect();

    // Value-struct names: let the vtable generator tell `Option<ValueStruct>`
    // apart from the optional types that already have `FfiType` impls.
    let value_type_idents: Vec<syn::Ident> = parsed
        .entries
        .iter()
        .filter_map(|e| match &e.entry {
            LibraryEntry::Value(path) => Some(path_last_ident(path).clone()),
            _ => None,
        })
        .collect();

    for entry in &parsed.entries {
        let cfg = entry.cfg.as_ref();
        match &entry.entry {
            LibraryEntry::Tagged(path, tag) => {
                let last_ident = path_last_ident(path);
                let full_tag = (library_tag << 24) | tag;

                // Shim macro that injects the tag into the metadata macro call
                let alias = meta_alias_for_type(path);
                let alias_chain = to_chain_path(&alias);
                let shim_name = format_ident!("__ffier_tagged_{prefix_str}_{last_ident}");
                shim_macros.push(emit_library_shim(
                    &shim_name,
                    quote! {
                        #alias_chain! { $prefix, #full_tag, $callback $(, $($rest)*)? }
                    },
                ));

                shim_names.push(shim_name.clone());

                // @on_library_export generates FfiHandle + FfiType impls — must be
                // gated because these reference the concrete type which may not exist.
                reexport_invocations.push(maybe_cfg_wrap(
                    cfg,
                    quote! { #alias!(@on_library_export, #full_tag, [#(#handle_type_idents),*], [#(#value_type_idents),*]); },
                ));

                // Helper module re-export for qualified paths
                let helper_mod_name =
                    format_ident!("_ffier_{}", camel_to_snake(&last_ident.to_string()));
                if path.segments.len() > 1 {
                    let helper_mod_path = replace_last_segment(path, &helper_mod_name);
                    reexport_invocations.push(maybe_cfg_wrap(
                        cfg,
                        quote! {
                            #[doc(hidden)]
                            pub use #helper_mod_path;
                        },
                    ));
                }
            }
            LibraryEntry::TaggedTrait(path, tag) => {
                let last_ident = path_last_ident(path);
                let full_tag = (library_tag << 24) | tag;

                // The upstream metadata macro generates the wrapper type
                // via @on_library_export. The shim passes path overrides
                // (wrapper, vtable struct, trait path) using $crate:: which
                // resolves to the library crate. Downstream crates in the
                // chain only need the library crate, not the upstream crate.
                let alias = meta_alias_for_type(path);
                let wrapper_ident = format_ident!("Vtable{last_ident}");

                // Use `_trait_` in internal names to avoid collisions with
                // user types that share the same last segment (e.g. crate::Error
                // and trait ffier_builtins::Error).
                // Upstream alias — unconditional because the shim references
                // it during chain expansion regardless of whether the feature is on.
                let upstream_alias = format_ident!("__ffier_upstream_trait_{last_ident}");
                reexport_invocations.push(quote! {
                    #[doc(hidden)]
                    pub use #alias as #upstream_alias;
                });

                let is_external = path.segments.len() > 1
                    && path.segments.first().is_none_or(|seg| seg.ident != "crate");
                let trait_reexport = format_ident!("__ffier_reexport_trait_{last_ident}");

                if is_external {
                    // Trait reexport — unconditional (chain needs it).
                    reexport_invocations.push(quote! {
                        #[doc(hidden)]
                        pub use #path as #trait_reexport;
                    });

                    // Helper module re-export — cfg-gated (only exists when feature is on).
                    let trait_snake = camel_to_snake(&last_ident.to_string());
                    let helper_mod_name = format_ident!("_ffier_vtable_{trait_snake}");
                    let helper_mod_path = replace_last_segment(path, &helper_mod_name);
                    reexport_invocations.push(maybe_cfg_wrap(
                        cfg,
                        quote! {
                            #[doc(hidden)]
                            pub use #helper_mod_path;
                        },
                    ));
                }

                // Shim paths: wrapper and trait are at the library root (from @on_library_export)
                let shim_trait = if is_external {
                    quote! { $crate::#trait_reexport }
                } else {
                    quote! { $crate::#last_ident }
                };
                let shim_wrapper = quote! { $crate::#wrapper_ident };

                let shim_name = format_ident!("__ffier_tagged_trait_{prefix_str}_{last_ident}");
                shim_macros.push(emit_library_shim(
                    &shim_name,
                    quote! {
                        $crate::#upstream_alias! { $prefix, #full_tag,
                            (#shim_wrapper),
                            (#shim_trait),
                            $callback $(, $($rest)*)? }
                    },
                ));

                // @on_library_export generates the vtable struct + wrapper type + impls.
                reexport_invocations.push(maybe_cfg_wrap(
                    cfg,
                    quote! { #alias!(@on_library_export, #full_tag, [#(#handle_type_idents),*], [#(#value_type_idents),*]); },
                ));
                shim_names.push(shim_name.clone());
            }
            LibraryEntry::Enum(path) => {
                let last_ident = path_last_ident(path);
                let alias = meta_alias_for_type(path);
                let alias_chain = to_chain_path(&alias);

                // Shim macro — no type tag needed for enums
                let shim_name = format_ident!("__ffier_enum_{prefix_str}_{last_ident}");
                shim_macros.push(emit_library_shim(
                    &shim_name,
                    quote! {
                        #alias_chain! { $prefix, 0, $callback $(, $($rest)*)? }
                    },
                ));

                // @on_library_export generates the FfiType impl for the enum
                reexport_invocations.push(maybe_cfg_wrap(
                    cfg,
                    quote! { #alias!(@on_library_export, 0u32, [#(#handle_type_idents),*], [#(#value_type_idents),*]); },
                ));
                shim_names.push(shim_name.clone());

                // Helper module re-export for qualified paths
                let helper_mod_name =
                    format_ident!("_ffier_{}", camel_to_snake(&last_ident.to_string()));
                if path.segments.len() > 1 {
                    let helper_mod_path = replace_last_segment(path, &helper_mod_name);
                    reexport_invocations.push(maybe_cfg_wrap(
                        cfg,
                        quote! {
                            #[doc(hidden)]
                            pub use #helper_mod_path;
                        },
                    ));
                }
            }
            LibraryEntry::Bitflags(path) => {
                let last_ident = path_last_ident(path);
                let alias = meta_alias_for_type(path);
                let alias_chain = to_chain_path(&alias);

                // Shim macro — no type tag needed for bitflags
                let shim_name = format_ident!("__ffier_bitflags_{prefix_str}_{last_ident}");
                shim_macros.push(emit_library_shim(
                    &shim_name,
                    quote! {
                        #alias_chain! { $prefix, 0, $callback $(, $($rest)*)? }
                    },
                ));

                // @on_library_export generates the FfiType impl for the bitflags type
                reexport_invocations.push(maybe_cfg_wrap(
                    cfg,
                    quote! { #alias!(@on_library_export, 0u32, [#(#handle_type_idents),*], [#(#value_type_idents),*]); },
                ));
                shim_names.push(shim_name.clone());

                // Helper module re-export for qualified paths
                let helper_mod_name =
                    format_ident!("_ffier_{}", camel_to_snake(&last_ident.to_string()));
                if path.segments.len() > 1 {
                    let helper_mod_path = replace_last_segment(path, &helper_mod_name);
                    reexport_invocations.push(maybe_cfg_wrap(
                        cfg,
                        quote! {
                            #[doc(hidden)]
                            pub use #helper_mod_path;
                        },
                    ));
                }
            }
            LibraryEntry::Value(path) => {
                let last_ident = path_last_ident(path);
                let alias = value_meta_alias(path);
                let alias_chain = to_chain_path(&alias);

                // Shim macro — value structs carry no type tag.
                let shim_name = format_ident!("__ffier_value_{prefix_str}_{last_ident}");
                shim_macros.push(emit_library_shim(
                    &shim_name,
                    quote! {
                        #alias_chain! { $prefix, 0, $callback $(, $($rest)*)? }
                    },
                ));

                // @on_library_export generates the FfiType impls.
                reexport_invocations.push(maybe_cfg_wrap(
                    cfg,
                    quote! { #alias!(@on_library_export, 0u32, [#(#handle_type_idents),*], [#(#value_type_idents),*]); },
                ));
                shim_names.push(shim_name.clone());

                // Helper module re-export for qualified paths
                let helper_mod_name =
                    format_ident!("_ffier_value_{}", camel_to_snake(&last_ident.to_string()));
                if path.segments.len() > 1 {
                    let helper_mod_path = replace_last_segment(path, &helper_mod_name);
                    reexport_invocations.push(maybe_cfg_wrap(
                        cfg,
                        quote! {
                            #[doc(hidden)]
                            pub use #helper_mod_path;
                        },
                    ));
                }
            }
            LibraryEntry::FreeFn(path) => {
                let last_ident = path_last_ident(path);
                let alias = meta_alias_for_type(path);
                let alias_chain = to_chain_path(&alias);

                // Shim macro — no type tag needed for free functions
                let shim_name = format_ident!("__ffier_fn_{prefix_str}_{last_ident}");
                shim_macros.push(emit_library_shim(
                    &shim_name,
                    quote! {
                        #alias_chain! { $prefix, 0, $callback $(, $($rest)*)? }
                    },
                ));

                shim_names.push(shim_name.clone());

                // Helper module re-export for qualified paths
                let helper_mod_name = format_ident!("_ffier_fn_{}", last_ident);
                if path.segments.len() > 1 {
                    let helper_mod_path = replace_last_segment(path, &helper_mod_name);
                    reexport_invocations.push(maybe_cfg_wrap(
                        cfg,
                        quote! {
                            #[doc(hidden)]
                            pub use #helper_mod_path;
                        },
                    ));
                }
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
                let alias_chain = to_chain_path(&alias);

                // Determine the trait path to pass through to the metadata.
                // For external traits, use the internal alias to avoid bare-name
                // collisions (e.g. user's `Error` enum vs builtin `Error` trait).
                // Check both the path itself AND the external_trait_names set
                // (handles `Error for TestError` where the trait is bare `Error`
                // but was registered as `trait ffier_builtins::Error = 25`).
                let is_external = external_trait_names.contains(&trait_name.to_string())
                    || (trait_path.segments.len() > 1
                        && trait_path
                            .segments
                            .first()
                            .is_none_or(|seg| seg.ident != "crate"));
                let resolved_trait_path = if is_external {
                    let reexport = format_ident!("__ffier_reexport_trait_{trait_name}");
                    quote! { $crate::#reexport }
                } else {
                    quote! { $crate::#trait_name }
                };

                // Shim macro that passes the resolved trait path to the
                // trait_impl metadata macro.
                let shim_name =
                    format_ident!("__ffier_trait_impl_{prefix_str}_{trait_name}_for_{struct_name}");
                shim_macros.push(emit_library_shim(
                    &shim_name,
                    quote! {
                        #alias_chain! { $prefix, (#resolved_trait_path),
                            $callback $(, $($rest)*)? }
                    },
                ));
                shim_names.push(shim_name.clone());

                // Helper module re-export for qualified paths
                let trait_snake = camel_to_snake(&trait_name.to_string());
                let struct_snake = camel_to_snake(&struct_name.to_string());
                let helper_mod_name = format_ident!("_ffier_impl_{trait_snake}_for_{struct_snake}");
                if struct_path.segments.len() > 1 {
                    let helper_mod_path = replace_last_segment(struct_path, &helper_mod_name);
                    reexport_invocations.push(maybe_cfg_wrap(
                        cfg,
                        quote! {
                            #[doc(hidden)]
                            pub use #helper_mod_path;
                        },
                    ));
                }
            }
        }
    }

    if shim_names.is_empty() {
        return quote! { compile_error!("library_definition! requires at least one type"); }.into();
    }

    let first = &shim_names[0];
    let rest = &shim_names[1..];

    let entry_macro_name = format_ident!("__ffier_{prefix_str}_metadata");

    let output = quote! {
        #[doc(hidden)]
        pub trait FfiHandle {
            const C_HANDLE_NAME: &'static str;
            const TYPE_TAG: u32;
        }

        #[doc(hidden)]
        pub trait FfiType {
            type CRepr;
            const C_TYPE_NAME: &'static str;
            const IS_HANDLE: bool;
            fn into_c(self) -> Self::CRepr;
            /// Reconstruct a value from its C representation.
            ///
            /// # Safety
            /// For handle types, `repr` must be a valid handle pointer.
            unsafe fn from_c(repr: Self::CRepr) -> Self;
        }

        /// Produce a CRepr from `&self` without consuming.
        /// Used by error payload getters to borrow data from an error handle.
        /// Only implemented for types that appear as error payload fields
        /// (primitives, enums, bitflags, strings, byte slices, paths).
        #[doc(hidden)]
        pub trait FfiBorrow: FfiType {
            fn borrow_as_c(&self) -> Self::CRepr;
        }

        #[doc(hidden)]
        pub use ffier::FfiError;

        macro_rules! __ffier_impl_ffi_identity {
            ($($rust_ty:ty => $c_name:expr),* $(,)?) => {
                $(impl FfiType for $rust_ty {
                    type CRepr = $rust_ty;
                    const C_TYPE_NAME: &'static str = $c_name;
                    const IS_HANDLE: bool = false;
                    fn into_c(self) -> Self { self }
                    unsafe fn from_c(repr: Self) -> Self { repr }
                }
                impl FfiBorrow for $rust_ty {
                    fn borrow_as_c(&self) -> Self { *self }
                })*
            };
        }
        __ffier_impl_ffi_identity! {
            i8 => "int8_t", i16 => "int16_t", i32 => "int32_t", i64 => "int64_t",
            u8 => "uint8_t", u16 => "uint16_t", u32 => "uint32_t", u64 => "uint64_t",
            isize => "ssize_t", usize => "size_t", bool => "bool",
            f32 => "float", f64 => "double",
        }

        // Opaque raw pointers — passed through without transformation.
        impl FfiType for *mut core::ffi::c_void {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &'static str = "void*";
            const IS_HANDLE: bool = false;
            fn into_c(self) -> Self { self }
            unsafe fn from_c(repr: Self) -> Self { repr }
        }

        impl FfiType for *const core::ffi::c_void {
            type CRepr = *const core::ffi::c_void;
            const C_TYPE_NAME: &'static str = "const void*";
            const IS_HANDLE: bool = false;
            fn into_c(self) -> Self { self }
            unsafe fn from_c(repr: Self) -> Self { repr }
        }

        impl FfiType for &str {
            type CRepr = ffier::FfierBytes;
            const C_TYPE_NAME: &'static str = "FfierStr";
            const IS_HANDLE: bool = false;
            fn into_c(self) -> ffier::FfierBytes { unsafe { ffier::FfierBytes::from_str(self) } }
            unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
                unsafe {
                    let bytes = core::slice::from_raw_parts(repr.data, repr.len);
                    core::str::from_utf8_unchecked(bytes)
                }
            }
        }
        impl FfiBorrow for &str {
            fn borrow_as_c(&self) -> ffier::FfierBytes { unsafe { ffier::FfierBytes::from_str(self) } }
        }

        impl<'a> FfiType for Option<&'a str> {
            type CRepr = ffier::FfierBytes;
            const C_TYPE_NAME: &'static str = "FfierStr";
            const IS_HANDLE: bool = false;
            fn into_c(self) -> ffier::FfierBytes {
                match self {
                    Some(s) => unsafe { ffier::FfierBytes::from_str(s) },
                    None => ffier::FfierBytes::EMPTY,
                }
            }
            unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
                if repr.data.is_null() {
                    None
                } else {
                    unsafe {
                        let bytes = core::slice::from_raw_parts(repr.data, repr.len);
                        Some(core::str::from_utf8_unchecked(bytes))
                    }
                }
            }
        }

        impl FfiType for Box<str> {
            type CRepr = ffier::FfierBytes;
            const C_TYPE_NAME: &'static str = "FfierStr";
            const IS_HANDLE: bool = false;
            fn into_c(self) -> ffier::FfierBytes {
                let leaked: &mut str = Box::leak(self);
                ffier::FfierBytes { data: leaked.as_mut_ptr() as *const u8, len: leaked.len() }
            }
            unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
                unsafe {
                    let slice = core::slice::from_raw_parts_mut(repr.data as *mut u8, repr.len);
                    Box::from_raw(core::str::from_utf8_unchecked_mut(slice))
                }
            }
        }
        impl FfiBorrow for Box<str> {
            fn borrow_as_c(&self) -> ffier::FfierBytes {
                ffier::FfierBytes { data: self.as_ptr(), len: self.len() }
            }
        }

        impl FfiType for &[u8] {
            type CRepr = ffier::FfierBytes;
            const C_TYPE_NAME: &'static str = "FfierBytes";
            const IS_HANDLE: bool = false;
            fn into_c(self) -> ffier::FfierBytes { unsafe { ffier::FfierBytes::from_bytes(self) } }
            unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
                unsafe {
                    if repr.data.is_null() { &[] }
                    else { core::slice::from_raw_parts(repr.data, repr.len) }
                }
            }
        }

        impl FfiType for &mut [u8] {
            type CRepr = ffier::FfierMutBytes;
            const C_TYPE_NAME: &'static str = "FfierBytesMut";
            const IS_HANDLE: bool = false;
            fn into_c(self) -> ffier::FfierMutBytes {
                unsafe { ffier::FfierMutBytes::from_bytes(self) }
            }
            unsafe fn from_c(repr: ffier::FfierMutBytes) -> Self {
                unsafe { repr.into_bytes() }
            }
        }

        impl FfiType for &std::path::Path {
            type CRepr = ffier::FfierBytes;
            const C_TYPE_NAME: &'static str = "FfierPath";
            const IS_HANDLE: bool = false;
            fn into_c(self) -> ffier::FfierBytes {
                unsafe { ffier::FfierBytes::from_path(self) }
            }
            unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
                unsafe { repr.as_path() }
            }
        }

        impl<'a> FfiType for Option<&'a std::path::Path> {
            type CRepr = ffier::FfierBytes;
            const C_TYPE_NAME: &'static str = "FfierPath";
            const IS_HANDLE: bool = false;
            fn into_c(self) -> ffier::FfierBytes {
                match self {
                    Some(p) => unsafe { ffier::FfierBytes::from_path(p) },
                    None => ffier::FfierBytes::EMPTY,
                }
            }
            unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
                if repr.data.is_null() {
                    None
                } else {
                    unsafe { Some(repr.as_path()) }
                }
            }
        }

        #[cfg(unix)]
        const _: () = {
            use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
            impl FfiType for OwnedFd {
                type CRepr = i32;
                const C_TYPE_NAME: &'static str = "int";
                const IS_HANDLE: bool = false;
                fn into_c(self) -> i32 { self.into_raw_fd() }
                unsafe fn from_c(fd: i32) -> Self { unsafe { OwnedFd::from_raw_fd(fd) } }
            }
            impl<'a> FfiType for BorrowedFd<'a> {
                type CRepr = i32;
                const C_TYPE_NAME: &'static str = "int";
                const IS_HANDLE: bool = false;
                fn into_c(self) -> i32 { self.as_raw_fd() }
                unsafe fn from_c(fd: i32) -> Self { unsafe { BorrowedFd::borrow_raw(fd) } }
            }
            impl<'a> FfiType for Option<BorrowedFd<'a>> {
                type CRepr = i32;
                const C_TYPE_NAME: &'static str = "int";
                const IS_HANDLE: bool = false;
                fn into_c(self) -> i32 {
                    match self {
                        Some(fd) => fd.as_raw_fd(),
                        None => -1,
                    }
                }
                unsafe fn from_c(fd: i32) -> Self {
                    if fd < 0 { None } else { Some(unsafe { BorrowedFd::borrow_raw(fd) }) }
                }
            }
            impl FfiType for Option<OwnedFd> {
                type CRepr = i32;
                const C_TYPE_NAME: &'static str = "int";
                const IS_HANDLE: bool = false;
                fn into_c(self) -> i32 {
                    match self {
                        Some(fd) => fd.into_raw_fd(),
                        None => -1,
                    }
                }
                unsafe fn from_c(fd: i32) -> Self {
                    if fd < 0 { None } else { Some(unsafe { OwnedFd::from_raw_fd(fd) }) }
                }
            }
        };

        #[cfg(windows)]
        const _: () = {
            use std::os::windows::io::{
                AsRawHandle, BorrowedHandle, FromRawHandle, IntoRawHandle, OwnedHandle, RawHandle,
            };
            impl FfiType for OwnedHandle {
                type CRepr = RawHandle;
                const C_TYPE_NAME: &'static str = "void *";
                const IS_HANDLE: bool = false;
                fn into_c(self) -> RawHandle { self.into_raw_handle() }
                unsafe fn from_c(handle: RawHandle) -> Self {
                    unsafe { OwnedHandle::from_raw_handle(handle) }
                }
            }
            impl<'a> FfiType for BorrowedHandle<'a> {
                type CRepr = RawHandle;
                const C_TYPE_NAME: &'static str = "void *";
                const IS_HANDLE: bool = false;
                fn into_c(self) -> RawHandle { self.as_raw_handle() }
                unsafe fn from_c(handle: RawHandle) -> Self {
                    unsafe { BorrowedHandle::borrow_raw(handle) }
                }
            }
            impl<'a> FfiType for Option<BorrowedHandle<'a>> {
                type CRepr = RawHandle;
                const C_TYPE_NAME: &'static str = "void *";
                const IS_HANDLE: bool = false;
                fn into_c(self) -> RawHandle {
                    self.map_or(core::ptr::null_mut(), |handle| handle.as_raw_handle())
                }
                unsafe fn from_c(handle: RawHandle) -> Self {
                    if handle.is_null() {
                        None
                    } else {
                        Some(unsafe { BorrowedHandle::borrow_raw(handle) })
                    }
                }
            }
            impl FfiType for Option<OwnedHandle> {
                type CRepr = RawHandle;
                const C_TYPE_NAME: &'static str = "void *";
                const IS_HANDLE: bool = false;
                fn into_c(self) -> RawHandle {
                    self.map_or(core::ptr::null_mut(), IntoRawHandle::into_raw_handle)
                }
                unsafe fn from_c(handle: RawHandle) -> Self {
                    if handle.is_null() {
                        None
                    } else {
                        Some(unsafe { OwnedHandle::from_raw_handle(handle) })
                    }
                }
            }
        };

        impl<T: FfiHandle + 'static> FfiType for &T {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
            const IS_HANDLE: bool = true;
            fn into_c(self) -> *mut core::ffi::c_void { unimplemented!("&T into_c") }
            unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self { unsafe { ffier::ffier_handle_borrow::<T>(repr) } }
        }

        impl<T: FfiHandle + 'static> FfiType for &mut T {
            type CRepr = *mut core::ffi::c_void;
            const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
            const IS_HANDLE: bool = true;
            fn into_c(self) -> *mut core::ffi::c_void { unimplemented!("&mut T into_c") }
            unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self { unsafe { ffier::ffier_handle_borrow_mut::<T>(repr) } }
        }

        // Create helper modules at the crate root and invoke @on_library_export arms.
        #(#reexport_invocations)*

        #(#shim_macros)*

        // Chain accumulator — shared by both `@cross_crate` and `@local`
        // metadata macro arms. The `$chain` parameter is how the chain
        // references itself: cross-crate passes `$crate::__ffier_chain`,
        // local passes the bare name `__ffier_chain` (rust#52234).
        #[doc(hidden)]
        #[macro_export]
        macro_rules! __ffier_chain {
            // Recursive: append metadata, call next shim
            ({ $($meta:tt)* }, $prefix:literal, $chain:path, $final_cb:path,
             $schema_output:literal,
             [$($acc:tt)*], [$next:path $(, $($remaining:path),*)?]) => {
                $next! { $prefix, $chain,
                    $prefix, $chain, $final_cb,
                    $schema_output,
                    [$($acc)* { $($meta)* }],
                    [$($($remaining),*)?]
                }
            };
            // Base case: all metadata accumulated, invoke bridge generator
            ({ $($meta:tt)* }, $prefix:literal, $chain:path, $final_cb:path,
             $schema_output:literal,
             [$($acc:tt)*], []) => {
                $final_cb! { @lib_crate = $crate; @primitives_prefix = #primitives_prefix_lit; @schema_output = $schema_output; $($acc)* { $($meta)* } }
            };
        }

        /// Metadata entry point for bridge generation.
        ///
        /// Not intended for direct use — call via [`ffier::generate_bridge!`]:
        /// ```ignore
        /// ffier::generate_bridge!(local = __ffier_{prefix}_metadata, schema_output = "target/ffier-{prefix}.json");
        /// ffier::generate_bridge!(external = mylib::__ffier_{prefix}_metadata, schema_output = "target/ffier-{prefix}.json");
        /// ```
        #[macro_export]
        macro_rules! #entry_macro_name {
            (@cross_crate, $schema_output:literal) => {
                $crate::#first! { #prefix_lit, $crate::__ffier_chain,
                    #prefix_lit, $crate::__ffier_chain, ffier::__generate_bridge,
                    $schema_output,
                    [],
                    [#($crate::#rest),*]
                }
            };
            (@local, $schema_output:literal) => {
                #first! { #prefix_lit, __ffier_chain,
                    #prefix_lit, __ffier_chain, ffier::__generate_bridge,
                    $schema_output,
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

/// Metadata alias emitted by a value-struct export.
///
/// Value structs get their own alias namespace: the same type may also carry an
/// `#[ffier::export]` impl block, whose alias is `__ffier_meta_{Name}`.
pub(crate) fn value_meta_alias(path: &syn::Path) -> syn::Path {
    let alias_ident = format_ident!("__ffier_meta_value_{}", path_last_ident(path));
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
    library_tag: u32,
    /// Optional override for primitive type names (Str, Bytes, Result, etc.).
    /// When set, primitive C types use this prefix instead of the library prefix.
    /// For example, `primitives_prefix = "krun"` produces `KrunStr` even if the
    /// library prefix is `"krun_init"`.
    primitives_prefix: Option<LitStr>,
    entries: Vec<ConditionalLibraryEntry>,
}

struct ConditionalLibraryEntry {
    cfg: Option<LibraryEntryCfg>,
    entry: LibraryEntry,
}

struct LibraryEntryCfg {
    predicates: Vec<syn::Meta>,
}

impl LibraryEntryCfg {
    fn enabled_attr(&self) -> proc_macro2::TokenStream {
        let predicate = self.predicate_tokens();
        quote! { #[cfg(#predicate)] }
    }

    fn predicate_tokens(&self) -> proc_macro2::TokenStream {
        if self.predicates.len() == 1 {
            let predicate = &self.predicates[0];
            quote! { #predicate }
        } else {
            let predicates = &self.predicates;
            quote! { all(#(#predicates),*) }
        }
    }
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
    /// An enum constant type (no type tag, value type): `enum Path`
    Enum(syn::Path),
    /// A bitflags type (no type tag, value type): `bitflags Path`
    Bitflags(syn::Path),
    /// A fixed-layout value struct (no type tag): `value Path`
    Value(syn::Path),
    /// A free function: `fn Path`
    FreeFn(syn::Path),
}

/// Parse a `= N` type tag after an identifier. Returns the tag value.
/// Rejects tag 0 (reserved for success / no-type) and tags that don't fit
/// in 24 bits (the upper 8 bits are reserved for the library tag).
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
    if tag > 0x00FF_FFFF {
        return Err(syn::Error::new(
            tag_lit.span(),
            "type tag must fit in 24 bits (max 16777215); \
             the upper 8 bits are reserved for library_tag",
        ));
    }
    Ok(tag)
}

fn parse_library_entry_cfg(input: syn::parse::ParseStream) -> syn::Result<Option<LibraryEntryCfg>> {
    let mut predicates = Vec::new();

    for attr in input.call(syn::Attribute::parse_outer)? {
        if !attr.path().is_ident("cfg") {
            return Err(syn::Error::new_spanned(
                attr,
                "library_definition! entries only support outer #[cfg(...)] attributes",
            ));
        }
        let predicate = attr.parse_args_with(cfg::parse_meta)?;
        cfg::from_meta(&predicate)?;
        predicates.push(predicate);
    }

    if predicates.is_empty() {
        Ok(None)
    } else {
        Ok(Some(LibraryEntryCfg { predicates }))
    }
}

fn maybe_cfg_wrap(
    cfg: Option<&LibraryEntryCfg>,
    tokens: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if let Some(cfg) = cfg {
        let attr = cfg.enabled_attr();
        quote! {
            #attr
            #tokens
        }
    } else {
        tokens
    }
}

fn emit_library_shim(
    shim_name: &syn::Ident,
    on_body: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    // The shim always forwards unconditionally. The metadata macro
    // always exists (emitted by the proc macro regardless of #[cfg]).
    // The cfg predicate is carried inside the metadata blob, not on the shim.
    // The @on_library_export invocations (FfiType impls, vtable generation)
    // are separately gated via maybe_cfg_wrap in library_definition!.
    quote! {
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #shim_name {
            ($prefix:literal, $callback:path $(, $($rest:tt)*)?) => {
                #on_body
            };
        }
    }
}

impl Parse for LibraryInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let prefix: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;

        // Parse required `library_tag = N`
        let lt_ident: syn::Ident = input.parse()?;
        if lt_ident != "library_tag" {
            return Err(syn::Error::new(
                lt_ident.span(),
                "expected `library_tag = N` (required, 1..=255)",
            ));
        }
        input.parse::<Token![=]>()?;
        let lt_lit: syn::LitInt = input.parse()?;
        let library_tag = lt_lit.base10_parse::<u32>()?;
        if library_tag == 0 {
            return Err(syn::Error::new(
                lt_lit.span(),
                "library_tag must be nonzero (0 would make tags \
                 indistinguishable from a library without a tag)",
            ));
        }
        if library_tag > 255 {
            return Err(syn::Error::new(
                lt_lit.span(),
                "library_tag must fit in 8 bits (max 255)",
            ));
        }
        input.parse::<Token![,]>()?;

        // Parse optional `primitives_prefix = "..."`.
        // Must appear immediately after library_tag if present.
        let primitives_prefix = if input.peek(syn::Ident) {
            let fork = input.fork();
            if fork
                .parse::<syn::Ident>()
                .is_ok_and(|id| id == "primitives_prefix")
            {
                let _: syn::Ident = input.parse()?; // consume "primitives_prefix"
                input.parse::<Token![=]>()?;
                let pp: LitStr = input.parse()?;
                input.parse::<Token![,]>()?;
                Some(pp)
            } else {
                None
            }
        } else {
            None
        };

        let mut entries = Vec::new();
        while !input.is_empty() {
            let cfg = parse_library_entry_cfg(input)?;
            let entry = if input.peek(Token![trait]) {
                // `trait Path = N`
                input.parse::<Token![trait]>()?;
                let path: syn::Path = input.parse()?;
                let tag = parse_type_tag(input)?;
                LibraryEntry::TaggedTrait(path, tag)
            } else if input.peek(Token![enum]) {
                // `enum Path`
                input.parse::<Token![enum]>()?;
                let path: syn::Path = input.parse()?;
                LibraryEntry::Enum(path)
            } else if input.peek(syn::Ident)
                && input
                    .fork()
                    .parse::<syn::Ident>()
                    .is_ok_and(|id| id == "bitflags")
            {
                // `bitflags Path`
                let _: syn::Ident = input.parse()?;
                let path: syn::Path = input.parse()?;
                LibraryEntry::Bitflags(path)
            } else if input.peek(syn::Ident)
                && input
                    .fork()
                    .parse::<syn::Ident>()
                    .is_ok_and(|id| id == "value")
            {
                // `value Path`
                let _: syn::Ident = input.parse()?;
                let path: syn::Path = input.parse()?;
                LibraryEntry::Value(path)
            } else if input.peek(Token![fn]) {
                // `fn Path`
                input.parse::<Token![fn]>()?;
                let path: syn::Path = input.parse()?;
                LibraryEntry::FreeFn(path)
            } else {
                let first: syn::Path = input.parse()?;
                if input.peek(Token![for]) {
                    // `TraitPath for StructPath`
                    input.parse::<Token![for]>()?;
                    let second: syn::Path = input.parse()?;
                    LibraryEntry::TraitImpl {
                        trait_path: first,
                        struct_path: second,
                    }
                } else {
                    // `Path = N`
                    let tag = parse_type_tag(input)?;
                    LibraryEntry::Tagged(first, tag)
                }
            };
            entries.push(ConditionalLibraryEntry { cfg, entry });
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(LibraryInput {
            prefix,
            library_tag,
            primitives_prefix,
            entries,
        })
    }
}
