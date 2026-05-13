use std::ffi::CStr;

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
    /// Used for runtime type identification of `void*` handles — both for
    /// type assertions (wrong-handle panics) and `impl Trait` dispatch.
    ///
    /// The same tag value is also used in `FfierError.code` (upper bits) for
    /// error type identification. Both mechanisms will eventually be unified,
    /// so tag numbers must be globally unique across all types in a library.
    const TYPE_TAG: u32;

    /// Returns the raw handle pointer for this value.
    ///
    /// - **Client side**: the wrapper struct holds `*mut c_void` directly.
    /// - **Library side**: recovers the `FfierHandleBox<T>` pointer via
    ///   `offset_of!`.
    ///
    /// # Safety
    /// On the library side, `self` must point into a valid `FfierHandleBox<Self>`.
    /// Calling this on a freestanding value (e.g. on the stack or in a `Vec`)
    /// produces a garbage pointer. Safety is enforced by the code generator:
    /// only generated bridge code calls `as_handle`, and only on references
    /// obtained from `FfierHandleBox` borrows.
    unsafe fn as_handle(&self) -> *mut c_void;
}

/// Every handle allocation is prefixed with a type tag and metadata so any
/// `void*` handle can be introspected at runtime.
///
/// The `metadata` field occupies what was previously alignment padding between
/// `type_tag: u32` and `value: T` (which typically starts at pointer alignment
/// on 64-bit systems). Zero additional space cost for pointer-aligned `T`.
#[repr(C)]
pub struct FfierHandleBox<T> {
    pub type_tag: u32,
    /// Metadata field. For vtable handles, bit 0 = 1 indicates the lower 16
    /// bits encode a method index for default-method dispatch skip:
    /// `metadata = 1 | (method_index << 1)`. For non-vtable types this is 0.
    pub metadata: u32,
    pub value: T,
}

/// Read the type tag from a raw handle pointer.
///
/// # Safety
/// `handle` must point to a valid `FfierHandleBox<_>`.
pub unsafe fn handle_type_tag(handle: *const core::ffi::c_void) -> u32 {
    unsafe { *(handle as *const u32) }
}

/// Read the metadata field from a raw handle pointer.
///
/// # Safety
/// `handle` must point to a valid `FfierHandleBox<_>`.
pub unsafe fn handle_metadata(handle: *const core::ffi::c_void) -> u32 {
    unsafe { *((handle as *const u32).add(1)) }
}

/// Vtable handle value — the `value` field of `FfierHandleBox<VtableHandle>`
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
// Blanket FfiType impls for handle references
// ---------------------------------------------------------------------------

impl<T: FfiHandle> FfiType for &T {
    type CRepr = *mut c_void;
    const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
    fn into_c(self) -> *mut c_void {
        // Safety: into_c is only called by generated bridge code on references
        // that point into a valid FfierHandleBox<T>.
        unsafe { self.as_handle() }
    }
    fn from_c(repr: *mut c_void) -> Self {
        unsafe { &(*(repr as *const FfierHandleBox<T>)).value }
    }
}

impl<T: FfiHandle> FfiType for &mut T {
    type CRepr = *mut c_void;
    const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
    fn into_c(self) -> *mut c_void {
        // Safety: into_c is only called by generated bridge code on references
        // that point into a valid FfierHandleBox<T>.
        unsafe { self.as_handle() }
    }
    fn from_c(repr: *mut c_void) -> Self {
        unsafe { &mut (*(repr as *mut FfierHandleBox<T>)).value }
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
/// impl is used for rich error messages cached inside the error handle.
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

// ---------------------------------------------------------------------------
// FfierErrorPayload --- real error boxed in FfierHandleBox with RTTI
// ---------------------------------------------------------------------------

/// Payload stored inside `FfierHandleBox<FfierErrorPayload<E>>` for error
/// handles. Preserves the concrete Rust error value alongside a cached
/// `Display` message.
///
/// `cached_msg` is first so `ft_error_message()` can read it without
/// knowing `E` — it's always at offset 0 of the payload regardless of
/// the error type's size.
#[repr(C)]
pub struct FfierErrorPayload<E> {
    /// Cached `Display::fmt()` output (first field — fixed offset).
    pub cached_msg: String,
    /// Pre-computed `FfierResult` (type_tag << 32 | code).
    pub result: FfierResult,
    /// The concrete Rust error value.
    pub error: E,
}

/// Box an `FfiError` value into a `FfierHandleBox`-based error handle.
///
/// The handle gets the error enum's type tag in the `FfierHandleBox` header,
/// making it a proper RTTI-tagged handle. The concrete error value `E` is
/// preserved for future `source()` chain traversal and downcasting.
pub fn ffier_error_box<E: FfiError>(error: E, type_tag: u32) -> *mut c_void {
    use std::fmt::Write;
    let mut cached_msg = String::new();
    let _ = write!(cached_msg, "{error}");
    let result = ffier_result(type_tag, error.code());
    let payload = FfierErrorPayload {
        cached_msg,
        result,
        error,
    };
    let boxed = Box::new(FfierHandleBox {
        type_tag,
        metadata: 0,
        value: payload,
    });
    Box::into_raw(boxed) as *mut c_void
}

/// Read the cached message from an error handle.
///
/// Works without knowing the concrete error type because `cached_msg`
/// is the first field of `FfierErrorPayload<E>` (`#[repr(C)]` — fixed
/// offset from the `FfierHandleBox` header).
///
/// # Safety
/// `handle` must be a valid error handle from `ffier_error_box`, or null.
pub unsafe fn ffier_error_message(handle: *const c_void) -> FfierBytes {
    if handle.is_null() {
        return FfierBytes::EMPTY;
    }
    // Handle points to start of FfierHandleBox { type_tag: u32, metadata: u32, value: ... }
    // value starts at offset 8. cached_msg is the first field of value (#[repr(C)]).
    let msg_ptr = unsafe { (handle as *const u8).add(8) as *const String };
    let msg = unsafe { &*msg_ptr };
    unsafe { FfierBytes::from_str(msg) }
}

/// Typed destroy — drops `FfierHandleBox<FfierErrorPayload<E>>`.
///
/// The caller (generated dispatch code) provides `E` via the type parameter.
/// This ensures `E`'s drop glue runs correctly.
///
/// # Safety
/// `handle` must be a valid error handle from `ffier_error_box::<E>`, or null.
pub unsafe fn ffier_error_destroy_typed<E>(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    // handle points to start of FfierHandleBox — cast directly.
    let box_ptr = handle as *mut FfierHandleBox<FfierErrorPayload<E>>;
    drop(unsafe { Box::from_raw(box_ptr) });
}
