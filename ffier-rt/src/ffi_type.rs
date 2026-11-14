//! Conversions between Rust types and their C representations.
//!
//! Every trait takes a token type parameter `Tok`. Each generated crate (a
//! library defined with `library_definition!`, or a generated Rust client)
//! defines its own `__FfierLocalToken` and implements these traits with it.
//! Because the token is local to that crate, the orphan rule lets the crate
//! implement the traits for types defined in other crates, such as a library
//! exporting an enum from a dependency.
//!
//! The built-in conversions below are implemented for every token, so
//! generated code passes its own token for every type without having to know
//! which types are built-in.

use core::ffi::c_void;
use std::path::Path;

use crate::{FfierBytes, FfierMutBytes, ffier_handle_borrow, ffier_handle_borrow_mut};

/// A Rust type with a C representation.
pub trait FfiType<Tok> {
    /// The C ABI representation.
    type CRepr;
    /// Crosses the ABI as an opaque handle pointer.
    const IS_HANDLE: bool = false;
}

/// Convert a Rust value into its C representation.
pub trait IntoC<Tok>: FfiType<Tok> {
    fn into_c(self) -> Self::CRepr;
}

/// Reconstruct a Rust value from its C representation.
pub trait FromC<Tok>: FfiType<Tok> {
    /// # Safety
    /// `repr` must be a valid C representation of this type; for handle
    /// types it must be a valid handle pointer.
    unsafe fn from_c(repr: Self::CRepr) -> Self;
}

/// Produce a C representation from `&self` without consuming it.
///
/// Used by error payload getters to borrow data from an error handle.
/// Only implemented for types that can appear as error payload fields.
pub trait FfiBorrow<Tok>: FfiType<Tok> {
    fn borrow_as_c(&self) -> Self::CRepr;
}

/// An opaque handle type.
pub trait FfiHandle<Tok> {
    /// The handle's name, for diagnostics and foreign-type checks.
    const C_HANDLE_NAME: &'static str;
    /// The library-qualified type tag stored in every handle.
    const TYPE_TAG: u32;
}

/// A handle whose Rust value lives inside the handle allocation (the
/// library side). A C handle pointer can be borrowed as `&T`.
pub trait NativeHandle<Tok>: FfiHandle<Tok> {}

/// A generated client wrapper around a handle pointer. `&T` passes the
/// pointer the wrapper holds.
pub trait ClientHandle<Tok>: FfiHandle<Tok> + Sized {
    /// The wrapped handle pointer, still owned by `self`.
    fn as_handle(&self) -> *mut c_void;
    /// Release the handle pointer without destroying it.
    fn into_raw(self) -> *mut c_void;
    /// Take ownership of a handle pointer.
    ///
    /// # Safety
    /// The handle must be valid, uniquely owned, and valid for every
    /// lifetime of `Self`.
    unsafe fn from_raw(handle: *mut c_void) -> Self;
}

/// Error payload access for an exported error type.
///
/// Implemented by `derive(FfiError)` in the error's defining crate, for every
/// token whose field conversions exist.
pub trait FfiErrorPayload<Tok> {
    /// Shallow-copy the indexed payload field's CRepr into `out_buf`.
    /// The written CRepr borrows from `&self`; the caller must not let it
    /// outlive the error value. Indices refer to native tuple positions,
    /// including opaque fields.
    ///
    /// # Panics
    /// Panics if the index is out of bounds, the field is opaque, or the
    /// buffer is too small.
    ///
    /// # Safety
    /// `out_buf` must be valid and aligned for the selected field's CRepr,
    /// with at least `buf_size` bytes.
    unsafe fn payload(&self, index: usize, out_buf: *mut c_void, buf_size: usize);
}

// ---------------------------------------------------------------------------
// Borrowed handles
// ---------------------------------------------------------------------------

impl<Tok, T: FfiHandle<Tok> + 'static> FfiType<Tok> for &T {
    type CRepr = *mut c_void;
    const IS_HANDLE: bool = true;
}

impl<Tok, T: FfiHandle<Tok> + 'static> FfiType<Tok> for &mut T {
    type CRepr = *mut c_void;
    const IS_HANDLE: bool = true;
}

impl<Tok, T: NativeHandle<Tok> + 'static> FromC<Tok> for &T {
    unsafe fn from_c(repr: *mut c_void) -> Self {
        unsafe { ffier_handle_borrow::<T>(repr) }
    }
}

impl<Tok, T: NativeHandle<Tok> + 'static> FromC<Tok> for &mut T {
    unsafe fn from_c(repr: *mut c_void) -> Self {
        unsafe { ffier_handle_borrow_mut::<T>(repr) }
    }
}

impl<Tok, T: ClientHandle<Tok> + 'static> IntoC<Tok> for &T {
    fn into_c(self) -> *mut c_void {
        self.as_handle()
    }
}

impl<Tok, T: ClientHandle<Tok> + 'static> IntoC<Tok> for &mut T {
    fn into_c(self) -> *mut c_void {
        self.as_handle()
    }
}

// ---------------------------------------------------------------------------
// Built-in types
// ---------------------------------------------------------------------------

/// Types whose Rust value is their own C representation.
macro_rules! impl_identity {
    ($($ty:ty),* $(,)?) => {$(
        impl<Tok> FfiType<Tok> for $ty {
            type CRepr = $ty;
        }
        impl<Tok> IntoC<Tok> for $ty {
            fn into_c(self) -> $ty { self }
        }
        impl<Tok> FromC<Tok> for $ty {
            unsafe fn from_c(repr: $ty) -> $ty { repr }
        }
    )*};
}

impl_identity!(
    i8,
    i16,
    i32,
    i64,
    u8,
    u16,
    u32,
    u64,
    isize,
    usize,
    bool,
    f32,
    f64,
    *mut c_void,
    *const c_void,
);

/// Primitives that can appear as error payload fields.
macro_rules! impl_identity_borrow {
    ($($ty:ty),* $(,)?) => {$(
        impl<Tok> FfiBorrow<Tok> for $ty {
            fn borrow_as_c(&self) -> $ty { *self }
        }
    )*};
}

impl_identity_borrow!(
    i8, i16, i32, i64, u8, u16, u32, u64, isize, usize, bool, f32, f64,
);

impl<Tok> FfiType<Tok> for &str {
    type CRepr = FfierBytes;
}
impl<Tok> IntoC<Tok> for &str {
    fn into_c(self) -> FfierBytes {
        unsafe { FfierBytes::from_str(self) }
    }
}
impl<Tok> FromC<Tok> for &str {
    unsafe fn from_c(repr: FfierBytes) -> Self {
        unsafe {
            let bytes = core::slice::from_raw_parts(repr.data, repr.len);
            core::str::from_utf8_unchecked(bytes)
        }
    }
}
impl<Tok> FfiBorrow<Tok> for &str {
    fn borrow_as_c(&self) -> FfierBytes {
        unsafe { FfierBytes::from_str(self) }
    }
}

/// `None` encodes as `FfierBytes::EMPTY` (null data pointer).
impl<Tok> FfiType<Tok> for Option<&str> {
    type CRepr = FfierBytes;
}
impl<Tok> IntoC<Tok> for Option<&str> {
    fn into_c(self) -> FfierBytes {
        match self {
            Some(s) => unsafe { FfierBytes::from_str(s) },
            None => FfierBytes::EMPTY,
        }
    }
}
impl<Tok> FromC<Tok> for Option<&str> {
    unsafe fn from_c(repr: FfierBytes) -> Self {
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

/// An owned string; the allocation transfers with the representation.
impl<Tok> FfiType<Tok> for Box<str> {
    type CRepr = FfierBytes;
}
impl<Tok> IntoC<Tok> for Box<str> {
    fn into_c(self) -> FfierBytes {
        let leaked: &mut str = Box::leak(self);
        FfierBytes {
            data: leaked.as_mut_ptr() as *const u8,
            len: leaked.len(),
        }
    }
}
impl<Tok> FromC<Tok> for Box<str> {
    unsafe fn from_c(repr: FfierBytes) -> Self {
        unsafe {
            let slice = core::slice::from_raw_parts_mut(repr.data as *mut u8, repr.len);
            Box::from_raw(core::str::from_utf8_unchecked_mut(slice))
        }
    }
}
impl<Tok> FfiBorrow<Tok> for Box<str> {
    fn borrow_as_c(&self) -> FfierBytes {
        FfierBytes {
            data: self.as_ptr(),
            len: self.len(),
        }
    }
}

impl<Tok> FfiType<Tok> for &[u8] {
    type CRepr = FfierBytes;
}
impl<Tok> IntoC<Tok> for &[u8] {
    fn into_c(self) -> FfierBytes {
        unsafe { FfierBytes::from_bytes(self) }
    }
}
impl<Tok> FromC<Tok> for &[u8] {
    unsafe fn from_c(repr: FfierBytes) -> Self {
        if repr.data.is_null() {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(repr.data, repr.len) }
        }
    }
}

impl<Tok> FfiType<Tok> for &mut [u8] {
    type CRepr = FfierMutBytes;
}
impl<Tok> IntoC<Tok> for &mut [u8] {
    fn into_c(self) -> FfierMutBytes {
        unsafe { FfierMutBytes::from_bytes(self) }
    }
}
impl<Tok> FromC<Tok> for &mut [u8] {
    unsafe fn from_c(repr: FfierMutBytes) -> Self {
        unsafe { repr.into_bytes() }
    }
}

/// OS paths reuse the byte representation of the platform `OsStr`.
impl<Tok> FfiType<Tok> for &Path {
    type CRepr = FfierBytes;
}
impl<Tok> IntoC<Tok> for &Path {
    fn into_c(self) -> FfierBytes {
        unsafe { FfierBytes::from_path(self) }
    }
}
impl<Tok> FromC<Tok> for &Path {
    unsafe fn from_c(repr: FfierBytes) -> Self {
        unsafe { repr.as_path() }
    }
}

/// `None` encodes as `FfierBytes::EMPTY` (null data pointer).
impl<Tok> FfiType<Tok> for Option<&Path> {
    type CRepr = FfierBytes;
}
impl<Tok> IntoC<Tok> for Option<&Path> {
    fn into_c(self) -> FfierBytes {
        match self {
            Some(p) => unsafe { FfierBytes::from_path(p) },
            None => FfierBytes::EMPTY,
        }
    }
}
impl<Tok> FromC<Tok> for Option<&Path> {
    unsafe fn from_c(repr: FfierBytes) -> Self {
        if repr.data.is_null() {
            None
        } else {
            unsafe { Some(repr.as_path()) }
        }
    }
}

/// File descriptors cross as `int`; `None` is `-1`.
#[cfg(unix)]
mod fd {
    use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

    use super::{FfiType, FromC, IntoC};

    impl<Tok> FfiType<Tok> for OwnedFd {
        type CRepr = RawFd;
    }
    impl<Tok> IntoC<Tok> for OwnedFd {
        fn into_c(self) -> RawFd {
            self.into_raw_fd()
        }
    }
    impl<Tok> FromC<Tok> for OwnedFd {
        unsafe fn from_c(fd: RawFd) -> Self {
            unsafe { OwnedFd::from_raw_fd(fd) }
        }
    }

    impl<Tok> FfiType<Tok> for Option<OwnedFd> {
        type CRepr = RawFd;
    }
    impl<Tok> IntoC<Tok> for Option<OwnedFd> {
        fn into_c(self) -> RawFd {
            self.map_or(-1, IntoRawFd::into_raw_fd)
        }
    }
    impl<Tok> FromC<Tok> for Option<OwnedFd> {
        unsafe fn from_c(fd: RawFd) -> Self {
            if fd < 0 {
                None
            } else {
                Some(unsafe { OwnedFd::from_raw_fd(fd) })
            }
        }
    }

    impl<Tok> FfiType<Tok> for BorrowedFd<'_> {
        type CRepr = RawFd;
    }
    impl<Tok> IntoC<Tok> for BorrowedFd<'_> {
        fn into_c(self) -> RawFd {
            self.as_raw_fd()
        }
    }
    impl<Tok> FromC<Tok> for BorrowedFd<'_> {
        unsafe fn from_c(fd: RawFd) -> Self {
            unsafe { BorrowedFd::borrow_raw(fd) }
        }
    }

    impl<Tok> FfiType<Tok> for Option<BorrowedFd<'_>> {
        type CRepr = RawFd;
    }
    impl<Tok> IntoC<Tok> for Option<BorrowedFd<'_>> {
        fn into_c(self) -> RawFd {
            self.map_or(-1, |fd| fd.as_raw_fd())
        }
    }
    impl<Tok> FromC<Tok> for Option<BorrowedFd<'_>> {
        unsafe fn from_c(fd: RawFd) -> Self {
            if fd < 0 {
                None
            } else {
                Some(unsafe { BorrowedFd::borrow_raw(fd) })
            }
        }
    }
}

/// Windows handles cross as `void *`; `None` is null.
#[cfg(windows)]
mod windows_handle {
    use std::os::windows::io::{
        AsRawHandle, BorrowedHandle, FromRawHandle, IntoRawHandle, OwnedHandle, RawHandle,
    };

    use super::{FfiType, FromC, IntoC};

    impl<Tok> FfiType<Tok> for OwnedHandle {
        type CRepr = RawHandle;
    }
    impl<Tok> IntoC<Tok> for OwnedHandle {
        fn into_c(self) -> RawHandle {
            self.into_raw_handle()
        }
    }
    impl<Tok> FromC<Tok> for OwnedHandle {
        unsafe fn from_c(handle: RawHandle) -> Self {
            unsafe { OwnedHandle::from_raw_handle(handle) }
        }
    }

    impl<Tok> FfiType<Tok> for Option<OwnedHandle> {
        type CRepr = RawHandle;
    }
    impl<Tok> IntoC<Tok> for Option<OwnedHandle> {
        fn into_c(self) -> RawHandle {
            self.map_or(core::ptr::null_mut(), IntoRawHandle::into_raw_handle)
        }
    }
    impl<Tok> FromC<Tok> for Option<OwnedHandle> {
        unsafe fn from_c(handle: RawHandle) -> Self {
            if handle.is_null() {
                None
            } else {
                Some(unsafe { OwnedHandle::from_raw_handle(handle) })
            }
        }
    }

    impl<Tok> FfiType<Tok> for BorrowedHandle<'_> {
        type CRepr = RawHandle;
    }
    impl<Tok> IntoC<Tok> for BorrowedHandle<'_> {
        fn into_c(self) -> RawHandle {
            self.as_raw_handle()
        }
    }
    impl<Tok> FromC<Tok> for BorrowedHandle<'_> {
        unsafe fn from_c(handle: RawHandle) -> Self {
            unsafe { BorrowedHandle::borrow_raw(handle) }
        }
    }

    impl<Tok> FfiType<Tok> for Option<BorrowedHandle<'_>> {
        type CRepr = RawHandle;
    }
    impl<Tok> IntoC<Tok> for Option<BorrowedHandle<'_>> {
        fn into_c(self) -> RawHandle {
            self.map_or(core::ptr::null_mut(), |handle| handle.as_raw_handle())
        }
    }
    impl<Tok> FromC<Tok> for Option<BorrowedHandle<'_>> {
        unsafe fn from_c(handle: RawHandle) -> Self {
            if handle.is_null() {
                None
            } else {
                Some(unsafe { BorrowedHandle::borrow_raw(handle) })
            }
        }
    }
}
