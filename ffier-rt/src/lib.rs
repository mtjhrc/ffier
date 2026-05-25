use std::ffi::CStr;

use core::ffi::c_void;

// ---------------------------------------------------------------------------
// Handle layout — heap-allocated box with RTTI header
// ---------------------------------------------------------------------------

/// The handle struct wrapping every value exported across FFI.
///
/// Always heap-allocated via `Box<FfierHandle<T>>`. The FFI pointer is
/// `*mut c_void` pointing to the `FfierHandle<T>`. All handle types —
/// regular exportable types, error types, vtable types — share this
/// exact layout, differing only in `T`.
#[repr(C)]
pub struct FfierHandle<T> {
    pub type_tag: u32,
    pub metadata: u32,
    pub value: T,
}

/// Byte offset from handle start to the value field.
pub const HANDLE_VALUE_OFFSET: usize = core::mem::offset_of!(FfierHandle<()>, value);

// ---------------------------------------------------------------------------
// Handle introspection
// ---------------------------------------------------------------------------

/// Read the type tag from a raw handle pointer (offset 0).
///
/// # Safety
/// `handle` must point to a valid `FfierHandle<T>`.
#[inline]
pub unsafe fn handle_type_tag(handle: *const c_void) -> u32 {
    unsafe { *(handle as *const u32) }
}

/// Read the metadata field from a raw handle pointer (offset 4).
///
/// # Safety
/// `handle` must point to a valid `FfierHandle<T>`.
#[inline]
pub unsafe fn handle_metadata(handle: *const c_void) -> u32 {
    unsafe { *((handle as *const u32).add(1)) }
}

// ---------------------------------------------------------------------------
// Handle allocation
// ---------------------------------------------------------------------------

/// Allocate a new handle on the heap and return a raw pointer.
///
/// The returned pointer must eventually be passed to `ffier_handle_drop`
/// or `ffier_handle_consume` to avoid leaking.
#[inline]
pub fn ffier_handle_new<T>(tag: u32, value: T) -> *mut c_void {
    debug_assert!(tag != 0, "type tag must be nonzero");
    let handle = Box::new(FfierHandle {
        type_tag: tag,
        metadata: 0,
        value,
    });
    Box::into_raw(handle) as *mut c_void
}

/// Allocate a new handle with custom metadata (e.g. for vtable handles).
#[inline]
pub fn ffier_handle_new_with_metadata<T>(tag: u32, metadata: u32, value: T) -> *mut c_void {
    debug_assert!(tag != 0, "type tag must be nonzero");
    let handle = Box::new(FfierHandle {
        type_tag: tag,
        metadata,
        value,
    });
    Box::into_raw(handle) as *mut c_void
}

// ---------------------------------------------------------------------------
// Handle borrow
// ---------------------------------------------------------------------------

/// Borrow a shared reference to the value inside a handle.
///
/// # Safety
/// - `handle` must point to a valid `FfierHandle<T>`.
/// - The handle must be alive for the lifetime of the returned reference.
#[inline]
pub unsafe fn ffier_handle_borrow<T>(handle: *const c_void) -> &'static T {
    debug_assert!(!handle.is_null(), "null handle");
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "uninitialized handle"
    );
    unsafe { &(*(handle as *const FfierHandle<T>)).value }
}

/// Borrow a mutable reference to the value inside a handle.
///
/// # Safety
/// - `handle` must point to a valid `FfierHandle<T>`.
/// - The caller must have exclusive access.
#[inline]
pub unsafe fn ffier_handle_borrow_mut<T>(handle: *mut c_void) -> &'static mut T {
    debug_assert!(!handle.is_null(), "null handle");
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "uninitialized handle"
    );
    unsafe { &mut (*(handle as *mut FfierHandle<T>)).value }
}

// ---------------------------------------------------------------------------
// Handle consume — take ownership, free the box
// ---------------------------------------------------------------------------

/// Consume a handle: move the value out and free the allocation.
///
/// # Safety
/// - `handle` must point to a valid `FfierHandle<T>` created by
///   `ffier_handle_new`.
/// - After this call the handle pointer is dangling.
#[inline]
pub unsafe fn ffier_handle_consume<T>(handle: *mut c_void) -> T {
    debug_assert!(!handle.is_null(), "consuming null handle");
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "consuming uninitialized handle"
    );
    let boxed = unsafe { Box::from_raw(handle as *mut FfierHandle<T>) };
    boxed.value
}

// ---------------------------------------------------------------------------
// Handle drop — drop the value and free the allocation
// ---------------------------------------------------------------------------

/// Drop the value inside a handle and free the allocation.
///
/// # Safety
/// - `handle` must point to a valid `FfierHandle<T>` created by
///   `ffier_handle_new`, or be null (no-op).
#[inline]
pub unsafe fn ffier_handle_drop<T>(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "dropping uninitialized handle"
    );
    drop(unsafe { Box::from_raw(handle as *mut FfierHandle<T>) });
}

// ---------------------------------------------------------------------------
// VtableHandle --- payload for #[ffier::implementable] trait handles
// ---------------------------------------------------------------------------

/// Vtable handle payload — stored inline in the handle's payload region
/// for `#[ffier::implementable]` trait handles.
#[repr(C)]
pub struct VtableHandle {
    pub vtable_ptr: *const c_void,
    pub user_data: *const c_void,
    /// Size of the vtable struct as provided by the caller (truncated to
    /// `u16`; max 65535 bytes — more than enough for any vtable). Used for
    /// forward/backward compatibility: fields at offsets beyond this size
    /// are treated as `None` (default dispatch). This allows older clients
    /// (smaller vtable) to work with newer libraries (larger vtable) and
    /// vice versa.
    pub vtable_size: u16,
}

impl VtableHandle {
    /// Read an `Option<extern "C" fn(...)>` vtable field with bounds checking.
    ///
    /// Returns `None` if the field extends beyond `self.vtable_size`,
    /// providing forward/backward-compatible default dispatch for fields
    /// not present in the vtable.
    ///
    /// # Safety
    /// - `vtable_ptr` must point to a valid vtable struct of at least
    ///   `self.vtable_size` bytes.
    /// - `field_offset` must be the correct offset of an `Option<F>` field.
    #[inline]
    pub unsafe fn field_or_none<F: Copy>(&self, field_offset: usize) -> Option<F> {
        if field_offset + core::mem::size_of::<Option<F>>() > self.vtable_size as usize {
            None
        } else {
            unsafe { *(self.vtable_ptr.byte_add(field_offset) as *const Option<F>) }
        }
    }
}

// ---------------------------------------------------------------------------
// FfierBytes --- zero-copy byte slice for C FFI (&[u8], &str, &Path)
// ---------------------------------------------------------------------------

/// `#[repr(C)]` byte slice passed across FFI. In C, each usage gets a
/// typedef (`Str`, `Bytes`, `Path`) from the same underlying struct.
#[derive(Clone, Copy)]
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
// FfiError --- trait for error enums exported via FFI
// ---------------------------------------------------------------------------

/// Trait for error enums exported across the FFI boundary.
///
/// Requires `std::error::Error` (which implies `Display`). The `Display`
/// impl is used for error messages streamed on demand via `WriteStr`.
pub trait FfiError: std::error::Error + Sized {
    /// Variant code (lower 32 bits of `FfierResult`).
    fn code(&self) -> u32;

    /// Static human-readable message for a variant code (no allocation).
    fn static_message(code: u32) -> &'static CStr;

    /// `(CONSTANT_NAME, value)` pairs for C `#define` generation.
    fn codes() -> &'static [(&'static str, u32)];
}

// ---------------------------------------------------------------------------
// FfierResult --- packed u64 error code (upper 32 = type tag, lower 32 = code)
// ---------------------------------------------------------------------------

/// Packed error result: `0` = success, nonzero = `(type_tag << 32) | code`.
///
/// Users compare against generated constants (`FT_ERROR_CALC_OVERFLOW` etc.).
/// The internal layout is an implementation detail.
pub type FfierResult = u64;

/// Build a `FfierResult` from a type tag and variant code.
#[inline]
pub fn ffier_result(type_tag: u32, code: u32) -> FfierResult {
    ((type_tag as u64) << 32) | (code as u64)
}

/// Extract the type tag from a `FfierResult` (upper 32 bits).
#[inline]
pub fn ffier_result_type_tag(r: FfierResult) -> u32 {
    (r >> 32) as u32
}

/// Extract the error code from a `FfierResult` (lower 32 bits).
#[inline]
pub fn ffier_result_code(r: FfierResult) -> u32 {
    r as u32
}

/// Success value.
pub const FFIER_RESULT_SUCCESS: FfierResult = 0;

/// Convert an error value into a `FfierResult`.
///
/// `type_tag` identifies the error enum (assigned in `library_definition!`).
#[inline]
pub fn ffier_result_from_err<E: FfiError>(e: &E, type_tag: u32) -> FfierResult {
    ffier_result(type_tag, e.code())
}
