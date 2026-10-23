//! Handle allocation and destruction.
//!
//! These functions own the memory side of handles: allocate one, consume it,
//! or drop it. They live in their own crate rather than in `ffier-rt` because
//! they decide which allocator is used, and a binding must allocate in the
//! same place it frees. `ffier-rt` keeps the layout, the metadata bits, and
//! the read-only accessors, so a crate that only needs to inspect or borrow a
//! handle does not pull in allocation. The `ffier` facade re-exports these, so
//! generated code keeps naming `ffier::`.
//!
//! Every function here assumes the caller is the library that allocates the
//! handle, or code running inside it. A caller on the other side of the FFI
//! boundary cannot know which allocator owns a handle and must go through the
//! generated C functions instead.

use core::ffi::c_void;

use ffier_rt::{
    FfierBorrowedHandle, FfierHandle, FfierObjectArray, METADATA_DROP_VALUE,
    METADATA_FFIER_ALLOCATED, METADATA_INDIRECT,
};

/// Allocate a new owned handle on the heap and return a raw pointer.
///
/// Sets [`METADATA_FFIER_ALLOCATED`] and [`METADATA_DROP_VALUE`] —
/// [`ffier_handle_drop`] will run `T::drop` and deallocate. The returned
/// pointer must eventually be passed to [`ffier_handle_drop`] or
/// [`ffier_handle_consume`] to avoid leaking.
#[inline]
pub fn ffier_handle_new<T>(tag: u32, value: T) -> *mut c_void {
    debug_assert!(tag != 0, "type tag must be nonzero");
    let handle = Box::new(FfierHandle {
        type_tag: tag,
        metadata: METADATA_FFIER_ALLOCATED | METADATA_DROP_VALUE,
        value,
    });
    Box::into_raw(handle) as *mut c_void
}

/// Allocate an indirect handle: stores a raw pointer to the real `T` in
/// a 16-byte `FfierBorrowedHandle` (same header layout as `FfierHandle`).
///
/// The returned handle has a valid type_tag so it can be passed to methods
/// that type-check the handle. The value is NOT copied — the pointer is stored
/// directly. On destroy, the 16-byte handle is deallocated without touching
/// the pointed-to value.
///
/// # Safety
/// - `value` must point to a valid, aligned `T`.
/// - The caller must ensure the source `T` outlives all uses of this handle.
#[inline]
pub unsafe fn ffier_handle_new_borrowed<T>(tag: u32, value: *const T) -> *mut c_void {
    debug_assert!(tag != 0, "type tag must be nonzero");
    let handle = Box::new(FfierBorrowedHandle {
        type_tag: tag,
        metadata: METADATA_FFIER_ALLOCATED | METADATA_INDIRECT,
        ptr: value as *const c_void,
    });
    Box::into_raw(handle) as *mut c_void
}

/// Consume a handle: move the value out and free the allocation.
///
/// Only valid for inline handles that ffier allocated. Panics in all builds
/// when the handle is indirect, foreign, null, or uninitialized: an indirect
/// value is owned elsewhere, and a foreign handle may have been allocated
/// anywhere, so freeing either through Rust's allocator would be unsound.
///
/// # Safety
/// - `handle` must point to a valid, heap-allocated `FfierHandle<T>` created
///   by ffier (so [`METADATA_FFIER_ALLOCATED`] is set, [`METADATA_INDIRECT`]
///   is clear, and the allocation came from Rust's allocator).
/// - After this call the handle pointer is dangling.
#[inline]
pub unsafe fn ffier_handle_consume<T>(handle: *mut c_void) -> T {
    assert!(!handle.is_null(), "consuming null handle");
    assert!(
        unsafe { ffier_rt::handle_type_tag(handle) } != 0,
        "consuming uninitialized handle"
    );
    let meta = unsafe { ffier_rt::handle_metadata(handle) };
    assert!(
        meta & METADATA_INDIRECT == 0,
        "cannot consume an indirect handle: its value is owned elsewhere"
    );
    assert!(
        meta & METADATA_FFIER_ALLOCATED != 0,
        "cannot consume a foreign handle: only handles allocated by ffier \
         may be consumed, and freeing foreign memory through Rust's \
         allocator would be unsound"
    );
    let boxed = unsafe { Box::from_raw(handle as *mut FfierHandle<T>) };
    boxed.value
}

/// Drop a handle and free its allocation.
///
/// Requires [`METADATA_FFIER_ALLOCATED`]: only a handle ffier allocated may be
/// destroyed. A caller-owned handle or an object-array element panics, because
/// freeing either here would be unsound or would double free with the array.
///
/// With the allocation bit set, [`METADATA_DROP_VALUE`] decides whether the
/// value's destructor runs:
///
/// - set: frees the `Box<FfierHandle<T>>`, dropping the inline value.
/// - clear: frees the 16-byte `Box<FfierBorrowedHandle>` and leaves the
///   pointee alone.
///
/// NOTE: an indirect handle that sets [`METADATA_DROP_VALUE`] would describe a
/// pointee ffier owns. No constructor produces it, so this function asserts
/// rather than freeing the handle and leaking the pointee. Supporting it means
/// dropping through the pointer before freeing the handle; the payload layout
/// alone does not decide destruction.
///
/// # Safety
/// - `handle` must point to a valid, Box-allocated `FfierHandle<T>` or
///   `FfierBorrowedHandle`, or be null (no-op).
#[inline]
pub unsafe fn ffier_handle_drop<T>(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    debug_assert!(
        unsafe { ffier_rt::handle_type_tag(handle) } != 0,
        "dropping uninitialized handle"
    );
    let meta = unsafe { ffier_rt::handle_metadata(handle) };
    assert!(
        meta & METADATA_FFIER_ALLOCATED != 0,
        "cannot destroy a handle ffier did not allocate — a caller-owned \
         handle must not be destroyed, and an object-array element is freed \
         with the whole array"
    );
    // Dropping an indirect handle while claiming its value must be dropped
    // would free the 16-byte handle and never reach the pointee. No producer
    // sets that pair yet, so treat it as a contract violation rather than
    // leaking silently. Supporting it means dropping the pointee through the
    // pointer before freeing the handle.
    assert!(
        meta & METADATA_INDIRECT == 0 || meta & METADATA_DROP_VALUE == 0,
        "cannot drop an indirect handle whose pointee ffier owns: dropping the \
         value behind the pointer is not implemented"
    );
    if meta & METADATA_DROP_VALUE != 0 {
        // The handle owns its value: run the destructor with the allocation.
        drop(unsafe { Box::from_raw(handle as *mut FfierHandle<T>) });
    } else {
        // Borrowed: dealloc the 16-byte handle and leave the pointee alone.
        drop(unsafe { Box::from_raw(handle as *mut FfierBorrowedHandle) });
    }
}

/// Free a contiguous object array allocated by this library.
///
/// # Safety
/// - `arr` must have been created by this library's bridge (or be
///   [`FfierObjectArray::EMPTY`]).
/// - Must only be called once per array.
#[inline]
pub unsafe fn ffier_object_array_free(arr: FfierObjectArray) {
    if !arr.is_empty() {
        let raw_slice =
            core::ptr::slice_from_raw_parts(arr.items(), arr.len()) as *mut [FfierBorrowedHandle];
        drop(unsafe { Box::from_raw(raw_slice) });
    }
}
