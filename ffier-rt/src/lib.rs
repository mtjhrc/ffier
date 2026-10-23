use std::ffi::CStr;

use core::ffi::c_void;

// ---------------------------------------------------------------------------
// Handle layout — heap-allocated box with RTTI header
// ---------------------------------------------------------------------------

// NOTE: This crate describes handles; it does not allocate them. Everything
// here reads a handle or describes its layout, so it cannot pick the wrong
// allocator. Allocation and destruction live in `ffier-bridge`, and only code
// inside the library that allocates a handle should call those. Callers on the
// other side of the FFI boundary cannot tell which allocator owns a handle and
// must use the generated C functions.

/// The handle struct wrapping every value exported across FFI.
///
/// Always heap-allocated via `Box<FfierHandle<T>>`. The FFI pointer is
/// `*mut c_void` pointing to the `FfierHandle<T>`. All handle types —
/// regular exported types, error types, vtable types — share this
/// exact layout, differing only in `T`.
#[repr(C)]
pub struct FfierHandle<T> {
    pub type_tag: u32,
    pub metadata: u32,
    pub value: T,
}

/// An indirect handle: same 8-byte header as `FfierHandle<T>`, but the
/// value field is a raw pointer back to the real `T` instead of an inline
/// copy. Fixed 16 bytes regardless of T size.
///
/// Allocated by `ffier_handle_new_borrowed`. The runtime functions check
/// [`METADATA_INDIRECT`] to decide whether the value is inline or reached
/// through the pointer, and [`METADATA_FFIER_ALLOCATED`] to decide whether
/// the handle allocation may be freed.
#[repr(C)]
pub struct FfierBorrowedHandle {
    pub type_tag: u32,
    pub metadata: u32,
    /// Raw pointer to the real value (lives in the parent struct).
    pub ptr: *const c_void,
}

/// Metadata bit 0: ffier allocated this handle. When set, ffier may free the
/// handle allocation. When clear, the allocation belongs to someone else — a
/// caller-owned handle or a `FfierObjectArray` element — and ffier must not
/// free it.
///
/// The bit covers the handle alone, never the value. Where the value lives is
/// [`METADATA_INDIRECT`], and whether dropping the handle also destroys the
/// value is [`METADATA_DROP_VALUE`]. `ffier_handle_new_borrowed` sets this bit
/// and [`METADATA_INDIRECT`] while leaving [`METADATA_DROP_VALUE`] clear: the
/// handle is ffier's allocation and the pointee is the caller's.
pub const METADATA_FFIER_ALLOCATED: u32 = 1;

/// Metadata bit 1: the value is reached indirectly. The handle is an
/// `FfierBorrowedHandle` whose payload field is a `*const c_void` to the real
/// `T`, which is owned elsewhere. When clear, the payload is an inline `T`.
///
/// Element state falls out of the two bits: `INDIRECT` without
/// [`METADATA_FFIER_ALLOCATED`] is a `FfierObjectArray` element, which is
/// freed by freeing the whole array rather than the element.
pub const METADATA_INDIRECT: u32 = 2;

/// Metadata bit 2: the value's destructor is meaningful for this handle, so
/// dropping the handle must run it.
///
/// This is a statement about the value, not about the allocation: the
/// allocation half is [`METADATA_FFIER_ALLOCATED`] and the layout half is
/// [`METADATA_INDIRECT`]. Setting it while [`METADATA_INDIRECT`] is also set
/// would describe a handle whose pointee ffier must drop, which no constructor
/// produces yet; `ffier_handle_drop` asserts against that combination instead
/// of silently leaking the pointee.
///
/// Cleared for borrowed shells and object-array elements, whose pointees
/// belong to someone else.
pub const METADATA_DROP_VALUE: u32 = 4;

/// Metadata bit 3: the handle carries default-dispatch re-entry metadata
/// for an exported trait method.
///
/// This bit reserves bits 4 through 19 for the trait method index. The index
/// field is defined only while this bit is set; when it is clear, those bits
/// are available for other flags, and bits 20 and above are always free.
/// Generated Rust client bindings use it to mark temporary vtable handles
/// created for defaulted methods, so the bridge can call the library default
/// helper instead of recursing back through the vtable.
pub const METADATA_DEFAULT_DISPATCH: u32 = 8;

/// Bit position where [`METADATA_DEFAULT_DISPATCH`] starts the method index
/// field. Bits below this position stay independent flags.
pub const METADATA_DEFAULT_DISPATCH_SHIFT: u32 = 4;

/// Mask covering the method index field while [`METADATA_DEFAULT_DISPATCH`]
/// is set. The field is `u16` wide, matching the vtable slot count.
pub const METADATA_DEFAULT_DISPATCH_METHOD_MASK: u32 =
    (u16::MAX as u32) << METADATA_DEFAULT_DISPATCH_SHIFT;

/// Encode metadata for a temporary handle used by default-method self-dispatch.
///
/// Panics when `method_index` does not fit in the `u16` index field.
#[inline]
pub fn default_dispatch_metadata(method_index: usize) -> u32 {
    let method_index = u16::try_from(method_index).expect("method index does not fit in u16");
    METADATA_DEFAULT_DISPATCH | (u32::from(method_index) << METADATA_DEFAULT_DISPATCH_SHIFT)
}

/// Decode the method index from default-dispatch metadata.
///
/// Returns `None` when the handle metadata does not carry the
/// [`METADATA_DEFAULT_DISPATCH`] flag, because the reserved bits only hold an
/// index while that flag is set. The field is masked to `u16` so unrelated
/// high bits cannot widen it.
#[inline]
pub fn default_dispatch_method_index(metadata: u32) -> Option<u16> {
    if metadata & METADATA_DEFAULT_DISPATCH == 0 {
        None
    } else {
        Some(
            ((metadata & METADATA_DEFAULT_DISPATCH_METHOD_MASK) >> METADATA_DEFAULT_DISPATCH_SHIFT)
                as u16,
        )
    }
}

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
// Handle borrow
// ---------------------------------------------------------------------------

/// Borrow a shared reference to the value inside a handle.
///
/// When [`METADATA_INDIRECT`] is clear the value is inline in the
/// `FfierHandle<T>`. When it is set the value field is a pointer in a
/// `FfierBorrowedHandle` that is followed to reach the real `T`.
///
/// # Safety
/// - `handle` must point to a valid `FfierHandle<T>` or `FfierBorrowedHandle`.
/// - The handle must be alive for the lifetime of the returned reference.
#[inline]
pub unsafe fn ffier_handle_borrow<T>(handle: *const c_void) -> &'static T {
    debug_assert!(!handle.is_null(), "null handle");
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "uninitialized handle"
    );
    if unsafe { handle_metadata(handle) } & METADATA_INDIRECT != 0 {
        let bh = unsafe { &*(handle as *const FfierBorrowedHandle) };
        unsafe { &*(bh.ptr as *const T) }
    } else {
        unsafe { &(*(handle as *const FfierHandle<T>)).value }
    }
}

/// Borrow a mutable reference to the value inside a handle.
///
/// Returns a reference to the inline value, or follows the stored pointer
/// when [`METADATA_INDIRECT`] is set.
///
/// # Safety
/// - `handle` must point to a valid `FfierHandle<T>` or `FfierBorrowedHandle`.
/// - The caller must have exclusive access.
#[inline]
pub unsafe fn ffier_handle_borrow_mut<T>(handle: *mut c_void) -> &'static mut T {
    debug_assert!(!handle.is_null(), "null handle");
    debug_assert!(
        unsafe { handle_type_tag(handle) } != 0,
        "uninitialized handle"
    );
    if unsafe { handle_metadata(handle) } & METADATA_INDIRECT != 0 {
        let bh = unsafe { &*(handle as *const FfierBorrowedHandle) };
        unsafe { &mut *(bh.ptr as *mut T) }
    } else {
        unsafe { &mut (*(handle as *mut FfierHandle<T>)).value }
    }
}

// ---------------------------------------------------------------------------
// VtableHandle --- payload for exported trait handles
// ---------------------------------------------------------------------------

/// Common 8-byte prefix of every exported trait vtable.
#[repr(C)]
pub struct VtableHeader {
    /// Number of virtual method slots (excluding drop), including reserved gaps.
    pub _num_slots: u16,
    /// Reserved for future use; initialize to zero.
    pub _reserved1: u16,
    /// Reserved for future use; initialize to zero.
    pub _reserved2: u32,
}

/// Vtable handle payload — stored inline in the handle's payload region
/// for exported trait handles.
#[repr(C)]
pub struct VtableHandle {
    pub vtable_ptr: *const c_void,
    pub user_data: *const c_void,
}

impl VtableHandle {
    /// Read an `Option<extern "C" fn(...)>` vtable slot with bounds checking.
    ///
    /// Slot 0 is always drop. Method index N occupies slot N + 1 and is
    /// present when N is less than the vtable header's `_num_slots`.
    /// Returns `None` for absent slots or null function pointers.
    ///
    /// # Safety
    /// - `vtable_ptr` must point to a valid, aligned [`VtableHeader`]
    ///   followed by an aligned, initialized drop slot and `_num_slots`
    ///   aligned, initialized virtual method slots.
    /// - `F` must be the function-pointer type of the requested slot.
    #[inline]
    pub unsafe fn field_or_none<F: Copy>(&self, slot: usize) -> Option<F> {
        let num_slots = unsafe { *(self.vtable_ptr as *const u16) };
        if slot > usize::from(num_slots) {
            None
        } else {
            let field_offset =
                core::mem::size_of::<VtableHeader>() + slot * core::mem::size_of::<Option<F>>();
            unsafe { *(self.vtable_ptr.byte_add(field_offset) as *const Option<F>) }
        }
    }
}

// ---------------------------------------------------------------------------
// FfierObjectArray --- contiguous array of borrowed handles
// ---------------------------------------------------------------------------

/// `#[repr(C)]` array of borrowed handles returned from methods that
/// produce `&[&T]` or `&[T]`. The backing storage is a single contiguous
/// `Box<[FfierBorrowedHandle]>` allocation, leaked to C as an opaque
/// pointer plus length.
///
/// Individual elements must NOT be passed to `destroy` — the whole array
/// is freed in one call via the array's destroy function, which
/// reconstructs and drops the `Box<[FfierBorrowedHandle]>`.
///
/// Access elements via `ffier_object_array_get()`, not direct indexing.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfierObjectArray {
    /// Opaque pointer to the first `FfierBorrowedHandle` element.
    /// Null when `len == 0`. NOT directly indexable from C —
    /// use `ffier_object_array_get()`.
    _opaque: *const FfierBorrowedHandle,
    pub len: usize,
}

impl FfierObjectArray {
    pub const EMPTY: Self = Self {
        _opaque: core::ptr::null(),
        len: 0,
    };

    /// Create from a leaked `Box<[FfierBorrowedHandle]>`.
    pub fn from_raw(ptr: *const FfierBorrowedHandle, len: usize) -> Self {
        Self { _opaque: ptr, len }
    }

    /// Pointer to the first element, or null when the array is empty.
    ///
    /// Reading it is allocation-free; only the owner of the array may free it.
    #[inline]
    pub fn items(&self) -> *const FfierBorrowedHandle {
        self._opaque
    }

    /// Number of elements in the array.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the array holds no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Get a handle pointer to the i-th element of a `FfierObjectArray`.
///
/// Returns a `*mut c_void` pointing to the `FfierBorrowedHandle` at
/// index `i`. The handle has valid RTTI headers and can be passed to
/// methods that type-check the handle.
///
/// # Safety
/// - `arr` must be a valid `FfierObjectArray`.
/// - `i` must be `< arr.len`.
#[inline]
pub unsafe fn ffier_object_array_get(arr: FfierObjectArray, i: usize) -> *mut c_void {
    debug_assert!(i < arr.len, "index out of bounds");
    unsafe { arr._opaque.add(i) as *mut c_void }
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

    /// Static message for a variant code (no allocation), defaulting to the
    /// raw variant name in derived implementations. Generated fieldless Display
    /// uses this message; native Display should match to avoid confusing text.
    fn static_message(code: u32) -> &'static CStr;

    /// `(CONSTANT_NAME, value)` pairs for C `#define` generation.
    fn codes() -> &'static [(&'static str, u32)];

    /// Construct a fieldless (unit or empty-tuple) variant from its code.
    /// Returns `None` for unknown codes and variants with payloads,
    /// including opaque payloads.
    fn from_code(_code: u32) -> Option<Self> {
        None
    }

    /// Shallow-copy the indexed payload field's CRepr into `out_buf`.
    /// The written CRepr borrows from `&self` — the caller must not
    /// outlive the error value. `buf_size` is checked for safety.
    /// Indices refer to native tuple positions, including opaque fields.
    ///
    /// # Panics
    /// Panics if the index is out of bounds, the field is opaque, or the buffer
    /// is too small. Fieldless variants have no valid field indices.
    ///
    /// # Safety
    /// `out_buf` must be valid and aligned for the selected field's CRepr,
    /// with at least `buf_size` bytes. Returned data must not outlive `self`.
    unsafe fn payload(&self, index: usize, _out_buf: *mut core::ffi::c_void, _buf_size: usize) {
        panic!("error payload field index {index} out of bounds for 0 fields");
    }
}

// ---------------------------------------------------------------------------
// FfierResult --- packed u64 error code (upper 32 = type tag, lower 32 = code)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Library tag + type tag composition
// ---------------------------------------------------------------------------

/// Number of bits reserved for the library tag in the upper part of `type_tag`.
pub const LIBRARY_TAG_BITS: u32 = 8;

/// Number of bits available for the per-type tag.
pub const TYPE_TAG_BITS: u32 = 32 - LIBRARY_TAG_BITS; // 24

/// Maximum valid library tag value (2^8 - 1 = 255).
pub const MAX_LIBRARY_TAG: u32 = (1 << LIBRARY_TAG_BITS) - 1;

/// Maximum valid per-type tag value (2^24 - 1 = 16_777_215).
pub const MAX_TYPE_TAG: u32 = (1 << TYPE_TAG_BITS) - 1;

/// Compose a full `type_tag` from a library tag and a per-type tag.
///
/// Layout: `[library_tag: 8 bits | type_tag: 24 bits]`
#[inline]
pub const fn compose_tag(library_tag: u32, type_tag: u32) -> u32 {
    (library_tag << TYPE_TAG_BITS) | type_tag
}

/// Extract the library tag (upper 8 bits) from a composed type tag.
#[inline]
pub const fn extract_library_tag(tag: u32) -> u32 {
    tag >> TYPE_TAG_BITS
}

/// Extract the per-type tag (lower 24 bits) from a composed type tag.
#[inline]
pub const fn extract_type_tag(tag: u32) -> u32 {
    tag & MAX_TYPE_TAG
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

/// Const-compatible string equality check.
pub const fn const_str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Success value.
pub const FFIER_RESULT_SUCCESS: FfierResult = 0;
