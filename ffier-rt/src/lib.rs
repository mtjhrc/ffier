use std::ffi::{CStr, CString, c_char};

// Re-export paste so that generated error bridge macros can use it.
pub use paste;

// ---------------------------------------------------------------------------
// FfiType --- maps Rust types to C-compatible representations
// ---------------------------------------------------------------------------

pub trait FfiType {
    type CRepr;
    const C_TYPE_NAME: &'static str;
    fn into_c(self) -> Self::CRepr;
    fn from_c(repr: Self::CRepr) -> Self;
}

macro_rules! impl_ffi_identity {
    ($($rust_ty:ty => $c_name:expr),* $(,)?) => {
        $(
            impl FfiType for $rust_ty {
                type CRepr = $rust_ty;
                const C_TYPE_NAME: &str = $c_name;
                fn into_c(self) -> Self { self }
                fn from_c(repr: Self) -> Self { repr }
            }
        )*
    };
}

impl_ffi_identity! {
    i8  => "int8_t",
    i16 => "int16_t",
    i32 => "int32_t",
    i64 => "int64_t",
    u8  => "uint8_t",
    u16 => "uint16_t",
    u32 => "uint32_t",
    u64 => "uint64_t",
    isize => "intptr_t",
    usize => "uintptr_t",
    bool => "bool",
}

#[cfg(feature = "std")]
mod std_impls {
    use super::FfiType;
    use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};

    impl FfiType for OwnedFd {
        type CRepr = i32;
        const C_TYPE_NAME: &str = "int";
        fn into_c(self) -> i32 {
            self.into_raw_fd()
        }
        fn from_c(fd: i32) -> Self {
            unsafe { OwnedFd::from_raw_fd(fd) }
        }
    }

    impl<'a> FfiType for BorrowedFd<'a> {
        type CRepr = i32;
        const C_TYPE_NAME: &'static str = "int";
        fn into_c(self) -> i32 {
            self.as_raw_fd()
        }
        fn from_c(fd: i32) -> Self {
            unsafe { BorrowedFd::borrow_raw(fd) }
        }
    }
}

// ---------------------------------------------------------------------------
// FfiType impls for reference types --- &str, &[u8], &Path → FfierBytes
// ---------------------------------------------------------------------------

impl<'a> FfiType for &'a str {
    type CRepr = FfierBytes;
    const C_TYPE_NAME: &'static str = "FfierStr";
    fn into_c(self) -> FfierBytes {
        unsafe { FfierBytes::from_str(self) }
    }
    fn from_c(repr: FfierBytes) -> Self {
        // Safety: FfierBytes holds a raw pointer into caller-owned data that
        // outlives this call. We reconstruct the reference with an unbounded
        // lifetime — only sound in generated bridge code where the source data
        // is guaranteed to be alive for the duration of the FFI call.
        unsafe {
            let bytes = core::slice::from_raw_parts(repr.data, repr.len);
            core::str::from_utf8_unchecked(bytes)
        }
    }
}

impl<'a> FfiType for &'a [u8] {
    type CRepr = FfierBytes;
    const C_TYPE_NAME: &'static str = "FfierBytes";
    fn into_c(self) -> FfierBytes {
        unsafe { FfierBytes::from_bytes(self) }
    }
    fn from_c(repr: FfierBytes) -> Self {
        unsafe {
            if repr.data.is_null() {
                &[]
            } else {
                core::slice::from_raw_parts(repr.data, repr.len)
            }
        }
    }
}

#[cfg(unix)]
impl<'a> FfiType for &'a std::path::Path {
    type CRepr = FfierBytes;
    const C_TYPE_NAME: &'static str = "FfierPath";
    fn into_c(self) -> FfierBytes {
        unsafe { FfierBytes::from_path(self) }
    }
    fn from_c(repr: FfierBytes) -> Self {
        use std::os::unix::ffi::OsStrExt;
        unsafe {
            let bytes = core::slice::from_raw_parts(repr.data, repr.len);
            std::path::Path::new(std::ffi::OsStr::from_bytes(bytes))
        }
    }
}

// ---------------------------------------------------------------------------
// FfiHandle --- marker for types exported via #[ffier::exportable]
// ---------------------------------------------------------------------------

use core::any::TypeId;
use core::ffi::c_void;

/// Marker trait for types that are exported as opaque C handles.
///
/// Automatically implemented by `#[ffier::exportable]`. Enables using
/// `&Widget` as a parameter type (borrows the handle) and `Widget` as
/// a return type (creates a new handle).
pub trait FfiHandle {
    /// The C handle typedef name (e.g. `"ExWidget"`).
    const C_HANDLE_NAME: &'static str;

    /// Returns the raw handle pointer for this value.
    ///
    /// - **Client side**: the wrapper struct holds `*mut c_void` directly.
    /// - **Library side**: recovers the `FfierTaggedBox<T>` pointer via
    ///   `offset_of!`.
    fn as_handle(&self) -> *mut c_void;

    /// Runtime type identifier. Only available for `'static` types.
    fn type_id() -> TypeId
    where
        Self: 'static,
    {
        TypeId::of::<Self>()
    }
}

/// Every handle allocation is prefixed with a type tag so any `void*`
/// handle can be introspected at runtime.
#[repr(C)]
pub struct FfierTaggedBox<T> {
    pub type_id: TypeId,
    pub value: T,
}

/// Read the TypeId from a raw handle pointer.
///
/// # Safety
/// `handle` must point to a valid `FfierTaggedBox<_>`.
pub unsafe fn handle_type_id(handle: *const core::ffi::c_void) -> TypeId {
    unsafe { *(handle as *const TypeId) }
}

// ---------------------------------------------------------------------------
// Blanket FfiType impls for handle references
// ---------------------------------------------------------------------------

impl<'a, T: FfiHandle> FfiType for &'a T {
    type CRepr = *mut c_void;
    const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
    fn into_c(self) -> *mut c_void {
        self.as_handle()
    }
    fn from_c(repr: *mut c_void) -> Self {
        unsafe { &(*(repr as *const FfierTaggedBox<T>)).value }
    }
}

impl<'a, T: FfiHandle> FfiType for &'a mut T {
    type CRepr = *mut c_void;
    const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
    fn into_c(self) -> *mut c_void {
        self.as_handle()
    }
    fn from_c(repr: *mut c_void) -> Self {
        unsafe { &mut (*(repr as *mut FfierTaggedBox<T>)).value }
    }
}

// ---------------------------------------------------------------------------
// FfierBytes --- zero-copy byte slice for C FFI (&[u8], &str, &Path)
// ---------------------------------------------------------------------------

/// `#[repr(C)]` byte slice passed across FFI. In C, each usage gets a
/// typedef (`Str`, `Bytes`, `Path`) from the same underlying struct.
#[repr(C)]
pub struct FfierBytes {
    pub data: *const u8,
    pub len: usize,
}

impl FfierBytes {
    pub const EMPTY: Self = Self {
        data: core::ptr::null(),
        len: 0,
    };

    /// # Safety
    /// `data` must be valid for `len` bytes, or null (returns `&[]`).
    pub unsafe fn as_bytes(&self) -> &[u8] {
        if self.data.is_null() {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(self.data, self.len) }
        }
    }

    /// # Safety
    /// `data` must point to valid UTF-8 of length `len`.
    pub unsafe fn as_str_unchecked(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
    }

    /// # Safety
    /// `data` must point to valid UTF-8 of length `len`.
    #[cfg(unix)]
    pub unsafe fn as_path(&self) -> &std::path::Path {
        use std::os::unix::ffi::OsStrExt;
        std::path::Path::new(std::ffi::OsStr::from_bytes(unsafe { self.as_bytes() }))
    }

    /// Create from a byte slice. The returned `FfierBytes` holds a raw pointer
    /// into `b`'s data — no copy is made.
    ///
    /// # Safety
    /// The caller must ensure the source data outlives the `FfierBytes` and any
    /// FFI call that receives it.
    pub unsafe fn from_bytes(b: &[u8]) -> Self {
        Self {
            data: b.as_ptr(),
            len: b.len(),
        }
    }

    /// Create from a string slice. The returned `FfierBytes` holds a raw pointer
    /// into `s`'s data — no copy is made.
    ///
    /// # Safety
    /// The caller must ensure the source string outlives the `FfierBytes` and any
    /// FFI call that receives it.
    pub unsafe fn from_str(s: &str) -> Self {
        unsafe { Self::from_bytes(s.as_bytes()) }
    }

    /// Create from a path. The returned `FfierBytes` holds a raw pointer
    /// into `p`'s data — no copy is made.
    ///
    /// # Safety
    /// The caller must ensure the source path outlives the `FfierBytes` and any
    /// FFI call that receives it.
    #[cfg(unix)]
    pub unsafe fn from_path(p: &std::path::Path) -> Self {
        use std::os::unix::ffi::OsStrExt;
        unsafe { Self::from_bytes(p.as_os_str().as_bytes()) }
    }
}

// ---------------------------------------------------------------------------
// FfiError --- per-type C error struct: { code, _msg }
// ---------------------------------------------------------------------------

pub trait FfiError: Sized {
    fn code(&self) -> u64;

    fn message(&self) -> Option<String> {
        None
    }

    fn static_message(code: u64) -> &'static CStr;

    /// `(CONSTANT_NAME, value)` pairs for C `#define` generation.
    fn codes() -> &'static [(&'static str, u64)];
}

#[repr(C)]
pub struct FfierError {
    pub code: u64,
    _msg: *mut c_char,
}

impl FfierError {
    pub fn ok() -> Self {
        Self {
            code: 0,
            _msg: core::ptr::null_mut(),
        }
    }

    pub fn from_err<E: FfiError>(e: E) -> Self {
        let code = e.code();
        let msg_ptr = match e.message() {
            Some(s) => CString::new(s)
                .map(CString::into_raw)
                .unwrap_or(core::ptr::null_mut()),
            None => core::ptr::null_mut(),
        };
        Self {
            code,
            _msg: msg_ptr,
        }
    }

    pub fn msg_ptr(&self) -> *const c_char {
        self._msg
    }

    /// # Safety
    /// `_msg` must be null or from `CString::into_raw`.
    pub unsafe fn free(&mut self) {
        if !self._msg.is_null() {
            drop(unsafe { CString::from_raw(self._msg) });
        }
        *self = Self::ok();
    }
}
