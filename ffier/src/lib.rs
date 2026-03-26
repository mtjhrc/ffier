use std::ffi::{CStr, CString, c_char};

pub use ffier_macros::FfiError;
pub use ffier_macros::exportable;
pub use ffier_macros::implementable;

// Re-export paste so that generated error bridge macros can use it.
pub use paste;

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

    impl FfiType for BorrowedFd<'static> {
        type CRepr = i32;
        const C_TYPE_NAME: &str = "int";
        fn into_c(self) -> i32 {
            self.as_raw_fd()
        }
        fn from_c(fd: i32) -> Self {
            unsafe { BorrowedFd::borrow_raw(fd) }
        }
    }
}

// ---------------------------------------------------------------------------
// FfiHandle — marker for types exported via #[ffier::exportable]
// ---------------------------------------------------------------------------

use core::any::TypeId;

/// Marker trait for types that are exported as opaque C handles.
///
/// Automatically implemented by `#[ffier::exportable]`. Enables using
/// `&Widget` as a parameter type (borrows the handle) and `Widget` as
/// a return type (creates a new handle).
pub trait FfiHandle: 'static {
    /// The C handle typedef name (e.g. `"ExWidget"`).
    const C_HANDLE_NAME: &str;

    /// Runtime type identifier.
    fn type_id() -> TypeId {
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

// ---------------------------------------------------------------------------
// HeaderSection / HeaderBuilder — structured C header generation
// ---------------------------------------------------------------------------

pub struct HeaderSection {
    pub struct_name: String,
    pub handle_typedef: String,
    pub shared_types: String,
    pub declarations: String,
}

pub struct HeaderBuilder {
    guard: String,
    sections: Vec<HeaderSection>,
}

// TODO: HeaderBuilder should accept the prefix (e.g. "krun") instead of a raw guard string,
// and derive the header guard, handle typedefs, shared type names, and macro names from it.
// The prefix should only be specified here, not duplicated in each #[ffier::exportable(prefix = "krun")].

impl HeaderBuilder {
    pub fn new(guard: &str) -> Self {
        Self {
            guard: guard.to_string(),
            sections: Vec::new(),
        }
    }

    pub fn add(mut self, section: HeaderSection) -> Self {
        self.sections.push(section);
        self
    }

    pub fn build(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!("#ifndef {}\n", self.guard));
        out.push_str(&format!("#define {}\n", self.guard));
        out.push('\n');
        out.push_str("#include <stdint.h>\n");
        out.push_str("#include <stdbool.h>\n");
        out.push_str("#include <string.h>\n");
        out.push('\n');

        // Collect all handle typedefs
        let mut has_handle = false;
        for section in &self.sections {
            if !section.handle_typedef.is_empty() {
                out.push_str(&section.handle_typedef);
                out.push('\n');
                has_handle = true;
            }
        }
        if has_handle {
            out.push('\n');
        }

        // Emit shared types from the first section that has them
        for section in &self.sections {
            if !section.shared_types.is_empty() {
                out.push_str(&section.shared_types);
                out.push('\n');
                break;
            }
        }

        out.push_str("/* Header auto-generated by ffier */\n");

        // Per-section declarations
        for section in &self.sections {
            if !section.declarations.is_empty() {
                // Ensure blank line before section comment
                if !out.ends_with("\n\n") {
                    if out.ends_with('\n') {
                        out.push('\n');
                    } else {
                        out.push_str("\n\n");
                    }
                }
                let name = &section.struct_name;
                let dashes = "-".repeat(72 - 6 - name.len()); // 72 cols total
                out.push_str(&format!("/* {name} {dashes} */\n\n"));
                out.push_str(&section.declarations);
            }
        }

        out.push('\n');
        out.push_str(&format!("#endif /* {} */\n", self.guard));
        out
    }
}
