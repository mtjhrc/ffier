use std::ffi::{CStr, CString, c_char};

pub use ffier_macros::reflect;

// ---------------------------------------------------------------------------
// FfiType — maps Rust types to C-compatible representations
// ---------------------------------------------------------------------------

pub trait FfiType {
    type CRepr;
    const C_TYPE_NAME: &str;
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
    use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};

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
}

// ---------------------------------------------------------------------------
// FfiError — per-type C error struct: { code, _msg }
// ---------------------------------------------------------------------------

/// Trait for Rust error types that cross the FFI boundary.
///
/// Each error type becomes its own C struct (typedef of `FfierError`),
/// with its own `_message()` and `_free()` functions.
pub trait FfiError: Sized {
    /// Error code for this value.
    fn code(&self) -> u64;

    /// Optional heap-allocated custom message. If `None`, `static_message()`
    /// provides the fallback (zero-allocation path).
    fn message(&self) -> Option<String> {
        None
    }

    /// Static fallback message for a given error code.
    fn static_message(code: u64) -> &'static CStr;

    /// `(CONSTANT_NAME, value)` pairs for C `#define` generation.
    fn codes() -> &'static [(&'static str, u64)];
}

/// The underlying `#[repr(C)]` error struct. Each user error type gets a
/// C typedef alias (e.g. `typedef FfierError CalcError;`).
#[repr(C)]
pub struct FfierError {
    pub code: u64,
    // Private — use the per-type `*_message()` function.
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

    /// Returns the custom message pointer (may be null).
    pub fn msg_ptr(&self) -> *const c_char {
        self._msg
    }
}

/// Free any heap-allocated message and zero the error struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ffier_error_free(err: *mut FfierError) {
    let err = unsafe { &mut *err };
    if !err._msg.is_null() {
        drop(unsafe { CString::from_raw(err._msg) });
    }
    *err = FfierError::ok();
}
