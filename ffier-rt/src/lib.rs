use std::ffi::CStr;

// ---------------------------------------------------------------------------
// FfiType --- maps Rust types to C-compatible representations
// ---------------------------------------------------------------------------

pub trait FfiType {
    type CRepr;
    const C_TYPE_NAME: &'static str;
    /// True for types backed by an opaque handle (`void*`).
    const IS_HANDLE: bool = false;
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

impl FfiType for &str {
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

impl FfiType for &[u8] {
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
impl FfiType for &std::path::Path {
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
// FfierBoxDyn --- newtype for dynamic dispatch of impl Trait params
// ---------------------------------------------------------------------------

/// Newtype around `Box<dyn T>` used for dynamic dispatch fallback.
///
/// When the combinatorial explosion of concrete `impl Trait` dispatch
/// exceeds the limit, params are wrapped into `FfierBoxDyn<dyn Trait>`
/// instead. `#[ffier::dispatch]` or `#[ffier::implementable]` generates
/// `impl Trait for FfierBoxDyn<dyn Trait>` which delegates to the inner
/// trait object.
pub struct FfierBoxDyn<T: ?Sized>(pub Box<T>);

// ---------------------------------------------------------------------------
// FfiHandle --- marker for types exported via #[ffier::exportable]
// ---------------------------------------------------------------------------

use core::ffi::c_void;

/// Marker trait for types that are exported as opaque C handles.
///
/// Automatically implemented by `#[ffier::exportable]`. Enables using
/// `&Widget` as a parameter type (borrows the handle) and `Widget` as
/// a return type (creates a new handle).
pub trait FfiHandle {
    /// The C handle typedef name (e.g. `"ExWidget"`).
    const C_HANDLE_NAME: &'static str;

    /// Stable numeric type tag assigned in `library_definition!`.
    ///
    /// Used for runtime type identification of handles — both for
    /// type assertions (wrong-handle panics) and `impl Trait` dispatch.
    ///
    /// The same tag value is also used in error result codes (upper bits)
    /// for error type identification. Tag numbers must be globally unique
    /// across all types in a library.
    const TYPE_TAG: u32;

    /// Returns the raw handle pointer for this value.
    ///
    /// - **Client side**: returns a pointer to the inline storage.
    /// - **Library side**: recovers the handle pointer by subtracting
    ///   the payload offset (8 bytes) from `self`.
    ///
    /// # Safety
    /// On the library side, `self` must point into a valid handle's
    /// payload region. Calling this on a freestanding value (e.g. on the
    /// stack or in a `Vec`) produces a garbage pointer. Safety is enforced
    /// by the code generator: only generated bridge code calls `as_handle`,
    /// and only on references obtained via `resolve()`.
    unsafe fn as_handle(&self) -> *mut c_void;
}

// ---------------------------------------------------------------------------
// Handle layout
// ---------------------------------------------------------------------------

/// The raw handle layout. Caller-owned storage has this shape; the `payload`
/// field is the minimum (one pointer) — actual handles may be larger.
#[repr(C)]
pub struct HandleLayout {
    pub type_tag: u32,
    pub metadata: u32,
    /// Minimum payload: one pointer (PTR mode fallback).
    pub payload: [u8; core::mem::size_of::<*const ()>()],
}

/// Bit 0 of the `metadata` field: 1 = value stored inline, 0 = pointer at
/// the payload offset (PTR mode). After zeroing, a handle is in PTR mode
/// with a NULL pointer — safe crash on misuse.
pub const INLINE_BIT: u32 = 1;

/// Byte offset from handle start to payload.
pub const HANDLE_PAYLOAD_OFFSET: usize = core::mem::offset_of!(HandleLayout, payload);

/// Minimum handle size in bytes (header + one-pointer payload).
pub const HANDLE_MIN_SIZE: usize = core::mem::size_of::<HandleLayout>();

// ---------------------------------------------------------------------------
// Handle introspection
// ---------------------------------------------------------------------------

/// Read the type tag from a raw handle pointer (offset 0).
///
/// # Safety
/// `handle` must point to a valid handle (at least 8 bytes, properly aligned).
#[inline]
pub unsafe fn handle_type_tag(handle: *const c_void) -> u32 {
    unsafe { *(handle as *const u32) }
}

/// Read the metadata field from a raw handle pointer (offset 4).
///
/// # Safety
/// `handle` must point to a valid handle (at least 8 bytes, properly aligned).
#[inline]
pub unsafe fn handle_metadata(handle: *const c_void) -> u32 {
    unsafe { *((handle as *const u32).add(1)) }
}

// ---------------------------------------------------------------------------
// Handle borrow --- read the value pointer from a handle (non-consuming)
// ---------------------------------------------------------------------------

/// Borrow a shared pointer to the value inside a handle.
///
/// Checks `INLINE_BIT` to determine whether the value is stored directly
/// in the payload (inline) or behind a heap pointer (PTR mode).
///
/// # Safety
/// - `handle` must point to a valid, initialized handle (type_tag != 0).
/// - The handle must contain a value of type `T`.
/// - The returned pointer is valid for the lifetime of the handle.
#[inline]
pub unsafe fn borrow_handle_ptr<T>(handle: *const c_void) -> *const T {
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "uninitialized handle"
    );
    let payload = unsafe { (handle as *const u8).add(HANDLE_PAYLOAD_OFFSET) };
    if unsafe { handle_metadata(handle) } & INLINE_BIT != 0 {
        payload as *const T
    } else {
        unsafe { *(payload as *const *const T) }
    }
}

/// Borrow a mutable pointer to the value inside a handle.
///
/// # Safety
/// Same as `borrow_handle_ptr`, plus the caller must have exclusive access.
#[inline]
pub unsafe fn borrow_handle_ptr_mut<T>(handle: *mut c_void) -> *mut T {
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "uninitialized handle"
    );
    let payload = unsafe { (handle as *mut u8).add(HANDLE_PAYLOAD_OFFSET) };
    if unsafe { handle_metadata(handle) } & INLINE_BIT != 0 {
        payload as *mut T
    } else {
        unsafe { *(payload as *const *mut T) }
    }
}

// ---------------------------------------------------------------------------
// Handle consume --- take ownership of the value, zero the handle
// ---------------------------------------------------------------------------

/// Consume a handle: move the value out and zero the entire handle.
///
/// After this call the handle is in a clean uninitialized state (all zeros).
/// If the value was in PTR mode, the heap allocation is freed.
///
/// # Safety
/// - `handle` must point to a valid, initialized handle containing a `T`.
/// - After this call the handle must not be used (it's zeroed).
#[inline]
pub unsafe fn consume_handle<T>(handle: *mut c_void) -> T {
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "consuming uninitialized handle"
    );
    let payload = unsafe { (handle as *mut u8).add(HANDLE_PAYLOAD_OFFSET) };
    let metadata = unsafe { handle_metadata(handle) };
    let value = if metadata & INLINE_BIT != 0 {
        unsafe { core::ptr::read(payload as *const T) }
    } else {
        let heap_ptr = unsafe { core::ptr::read(payload as *const *mut T) };
        unsafe { *Box::from_raw(heap_ptr) }
    };
    // Zero the handle — clean uninit state.
    unsafe { core::ptr::write_bytes(handle as *mut u8, 0, HANDLE_MIN_SIZE) };
    value
}

// ---------------------------------------------------------------------------
// Handle initialization
// ---------------------------------------------------------------------------

/// Initialize a handle in PTR mode: heap-allocates `value` and stores the
/// pointer at the payload offset.
///
/// # Safety
/// - `handle` must point to caller-owned storage with at least 16 bytes.
/// - `handle` must be uninitialized (type_tag == 0).
#[inline]
pub unsafe fn init_handle_ptr<T>(handle: *mut c_void, tag: u32, value: T) {
    debug_assert!(tag != 0, "type tag must be nonzero");
    let ptr = Box::into_raw(Box::new(value));
    unsafe {
        core::ptr::write(handle as *mut u32, tag);
        // metadata = 0 → PTR mode
        core::ptr::write((handle as *mut u32).add(1), 0);
        core::ptr::write(
            (handle as *mut u8).add(HANDLE_PAYLOAD_OFFSET) as *mut *mut T,
            ptr,
        );
    }
}

/// Initialize a handle in INLINE mode: writes `value` directly into the
/// payload region.
///
/// # Safety
/// - `handle` must point to caller-owned storage with at least
///   `8 + size_of::<T>()` bytes.
/// - `handle` must be uninitialized (type_tag == 0).
/// - `metadata` must have `INLINE_BIT` set. Caller may OR in additional
///   bits (e.g. vtable method index).
#[inline]
pub unsafe fn init_handle_inline<T>(handle: *mut c_void, tag: u32, metadata: u32, value: T) {
    debug_assert!(tag != 0, "type tag must be nonzero");
    debug_assert!(
        metadata & INLINE_BIT != 0,
        "init_handle_inline requires INLINE_BIT"
    );
    unsafe {
        core::ptr::write(handle as *mut u32, tag);
        core::ptr::write((handle as *mut u32).add(1), metadata);
        core::ptr::write(
            (handle as *mut u8).add(HANDLE_PAYLOAD_OFFSET) as *mut T,
            value,
        );
    }
}

/// Initialize a handle, choosing inline or PTR mode based on `buf_size`.
///
/// The caller must have written the available payload size as a `usize` at
/// offset 8 before calling this (the `KRUN_OUT()` macro does this). If
/// `buf_size >= size_of::<T>()`, the value is stored inline. Otherwise,
/// it is heap-allocated and a pointer is stored (PTR mode).
///
/// # Safety
/// - `handle` must point to caller-owned, zeroed storage.
/// - type_tag at offset 0 must be 0 (uninitialized).
/// - A `usize` at offset 8 must contain the available payload size.
/// - The storage must be at least `8 + max(size_of::<T>(), 8)` bytes
///   if inline, or at least 16 bytes if PTR fallback.
pub unsafe fn init_handle<T>(handle: *mut c_void, tag: u32, value: T) {
    debug_assert!(
        unsafe { handle_type_tag(handle) } == 0,
        "handle already initialized (double-init)"
    );
    let buf_size = unsafe {
        core::ptr::read((handle as *const u8).add(HANDLE_PAYLOAD_OFFSET) as *const usize)
    };
    if buf_size >= core::mem::size_of::<T>() {
        unsafe { init_handle_inline(handle, tag, INLINE_BIT, value) };
    } else {
        unsafe { init_handle_ptr(handle, tag, value) };
    }
}

// ---------------------------------------------------------------------------
// Handle drop
// ---------------------------------------------------------------------------

/// Drop the value in a handle and zero the type tag.
///
/// Checks `INLINE_BIT` to determine whether to `drop_in_place` (inline)
/// or `Box::from_raw` (PTR). Zeroing the tag ensures use-after-drop hits
/// the tag==0 assertion.
///
/// # Safety
/// - `handle` must point to a valid, initialized handle containing a `T`.
/// - After this call, the handle is uninitialized (tag == 0).
/// - The caller is responsible for freeing the outer storage if it was
///   heap-allocated.
pub unsafe fn drop_handle<T>(handle: *mut c_void) {
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "dropping uninitialized handle"
    );
    let payload = unsafe { (handle as *mut u8).add(HANDLE_PAYLOAD_OFFSET) };
    if unsafe { handle_metadata(handle) } & INLINE_BIT != 0 {
        unsafe { core::ptr::drop_in_place(payload as *mut T) };
    } else {
        let ptr = unsafe { core::ptr::read(payload as *const *mut T) };
        drop(unsafe { Box::from_raw(ptr) });
    }
    // Zero the handle — clean uninit state.
    unsafe { core::ptr::write_bytes(handle as *mut u8, 0, HANDLE_MIN_SIZE) };
}

// ---------------------------------------------------------------------------
// Blanket FfiType impls for handle references
// ---------------------------------------------------------------------------

impl<T: FfiHandle> FfiType for &T {
    type CRepr = *mut c_void;
    const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
    const IS_HANDLE: bool = true;
    fn into_c(self) -> *mut c_void {
        // Safety: into_c is only called by generated bridge code on references
        // that point into a valid handle's payload.
        unsafe { self.as_handle() }
    }
    fn from_c(repr: *mut c_void) -> Self {
        unsafe { &*borrow_handle_ptr::<T>(repr) }
    }
}

impl<T: FfiHandle> FfiType for &mut T {
    type CRepr = *mut c_void;
    const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
    const IS_HANDLE: bool = true;
    fn into_c(self) -> *mut c_void {
        // Safety: into_c is only called by generated bridge code on references
        // that point into a valid handle's payload.
        unsafe { self.as_handle() }
    }
    fn from_c(repr: *mut c_void) -> Self {
        unsafe { &mut *borrow_handle_ptr_mut::<T>(repr) }
    }
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
