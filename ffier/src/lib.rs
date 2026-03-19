use std::ffi::{CStr, CString, c_char};

pub use ffier_macros::exportable;

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
// FfierBytes — zero-copy byte slice for C FFI (&[u8], &str, &Path)
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

    pub fn from_bytes(b: &[u8]) -> Self {
        Self {
            data: b.as_ptr(),
            len: b.len(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        Self::from_bytes(s.as_bytes())
    }

    #[cfg(unix)]
    pub fn from_path(p: &std::path::Path) -> Self {
        use std::os::unix::ffi::OsStrExt;
        Self::from_bytes(p.as_os_str().as_bytes())
    }
}

// ---------------------------------------------------------------------------
// FfiError — per-type C error struct: { code, _msg }
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
