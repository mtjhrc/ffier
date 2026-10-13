#[cfg(unix)]
use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

/// Marker trait for types exported as opaque C handles.
pub trait FfiHandle {
    const C_HANDLE_NAME: &'static str;
    const TYPE_TAG: u32;
    /// # Safety
    /// The caller must ensure the handle is not used after the object is dropped.
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void;
    /// # Safety
    /// Handle must be valid, uniquely owned, and valid for every lifetime of Self.
    unsafe fn __from_raw(handle: *mut core::ffi::c_void) -> Self;
}

/// Maps Rust types to C-compatible representations.
pub trait FfiType {
    type CRepr;
    const C_TYPE_NAME: &'static str;
    const IS_HANDLE: bool = false;
    fn into_c(self) -> Self::CRepr;
    /// # Safety
    /// The C representation must be valid for this type.
    unsafe fn from_c(repr: Self::CRepr) -> Self;
}

/// Borrows layout-compatible Rust values from their C representations.
///
/// # Safety
/// Implementors must have the same layout as their `CRepr`.
pub unsafe trait BorrowFFiType: FfiType + Sized {
    /// # Safety
    /// `repr` must be non-null, aligned, initialized, and valid for the returned lifetime.
    unsafe fn borrow_from_c<'a>(repr: *const Self::CRepr) -> &'a Self {
        core::mem::forget(unsafe { Self::from_c(core::ptr::read(repr)) });
        unsafe { &*repr.cast::<Self>() }
    }
    /// # Safety
    /// `repr` must be non-null, aligned, initialized, exclusively borrowed, and valid for the returned lifetime.
    unsafe fn borrow_mut_from_c<'a>(repr: *mut Self::CRepr) -> &'a mut Self {
        core::mem::forget(unsafe { Self::from_c(core::ptr::read(repr)) });
        unsafe { &mut *repr.cast::<Self>() }
    }
}

macro_rules! impl_ffi_identity {
    ($($t:ty => $n:expr),* $(,)?) => { $(
        impl FfiType for $t {
            type CRepr = $t; const C_TYPE_NAME: &'static str = $n; const IS_HANDLE: bool = false;
            fn into_c(self) -> Self { self } unsafe fn from_c(r: Self) -> Self { r }
        }
    )* };
}
impl_ffi_identity! {
    i8 => "int8_t", i16 => "int16_t", i32 => "int32_t", i64 => "int64_t",
    u8 => "uint8_t", u16 => "uint16_t", u32 => "uint32_t", u64 => "uint64_t",
    isize => "ssize_t", usize => "size_t", bool => "bool", f32 => "float", f64 => "double",
    *mut core::ffi::c_void => "void*", *const core::ffi::c_void => "const void*",
}

impl FfiType for &str {
    type CRepr = ffier::FfierBytes;
    const C_TYPE_NAME: &'static str = "FfierStr";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> ffier::FfierBytes {
        unsafe { ffier::FfierBytes::from_str(self) }
    }
    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
        unsafe {
            let b = core::slice::from_raw_parts(repr.data, repr.len);
            core::str::from_utf8_unchecked(b)
        }
    }
}

impl FfiType for Option<&str> {
    type CRepr = ffier::FfierBytes;
    const C_TYPE_NAME: &'static str = "FfierStr";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> ffier::FfierBytes {
        match self {
            Some(s) => unsafe { ffier::FfierBytes::from_str(s) },
            None => ffier::FfierBytes::EMPTY,
        }
    }
    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
        if repr.data.is_null() {
            None
        } else {
            unsafe {
                Some(core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                    repr.data, repr.len,
                )))
            }
        }
    }
}

impl FfiType for Box<str> {
    type CRepr = ffier::FfierBytes;
    const C_TYPE_NAME: &'static str = "FfierStr";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> ffier::FfierBytes {
        let leaked: &mut str = Box::leak(self);
        ffier::FfierBytes {
            data: leaked.as_mut_ptr() as *const u8,
            len: leaked.len(),
        }
    }
    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
        unsafe {
            let slice = core::slice::from_raw_parts_mut(repr.data as *mut u8, repr.len);
            Box::from_raw(core::str::from_utf8_unchecked_mut(slice))
        }
    }
}

impl FfiType for &[u8] {
    type CRepr = ffier::FfierBytes;
    const C_TYPE_NAME: &'static str = "FfierBytes";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> ffier::FfierBytes {
        unsafe { ffier::FfierBytes::from_bytes(self) }
    }
    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {
        unsafe {
            if repr.data.is_null() {
                &[]
            } else {
                core::slice::from_raw_parts(repr.data, repr.len)
            }
        }
    }
}

impl FfiType for &mut [u8] {
    type CRepr = ffier::FfierMutBytes;
    const C_TYPE_NAME: &'static str = "FfierBytesMut";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> ffier::FfierMutBytes {
        unsafe { ffier::FfierMutBytes::from_bytes(self) }
    }
    unsafe fn from_c(repr: ffier::FfierMutBytes) -> Self {
        unsafe { repr.into_bytes() }
    }
}

#[cfg(unix)]
impl FfiType for OwnedFd {
    type CRepr = RawFd;
    const C_TYPE_NAME: &'static str = "int";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> RawFd {
        use std::os::unix::io::IntoRawFd;
        self.into_raw_fd() as RawFd
    }
    unsafe fn from_c(fd: RawFd) -> Self {
        unsafe { OwnedFd::from_raw_fd(fd as _) }
    }
}

#[cfg(unix)]
impl FfiType for Option<OwnedFd> {
    type CRepr = RawFd;
    const C_TYPE_NAME: &'static str = "int";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> RawFd {
        use std::os::unix::io::IntoRawFd;
        match self {
            Some(fd) => fd.into_raw_fd() as RawFd,
            None => -1,
        }
    }
    unsafe fn from_c(fd: RawFd) -> Self {
        if fd < 0 {
            None
        } else {
            Some(unsafe { OwnedFd::from_raw_fd(fd as _) })
        }
    }
}

#[cfg(unix)]
impl<'fd> FfiType for BorrowedFd<'fd> {
    type CRepr = RawFd;
    const C_TYPE_NAME: &'static str = "int";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> RawFd {
        self.as_raw_fd() as RawFd
    }
    unsafe fn from_c(fd: RawFd) -> Self {
        unsafe { BorrowedFd::borrow_raw(fd as _) }
    }
}

#[cfg(unix)]
impl<'fd> FfiType for Option<BorrowedFd<'fd>> {
    type CRepr = RawFd;
    const C_TYPE_NAME: &'static str = "int";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> RawFd {
        match self {
            Some(fd) => fd.as_raw_fd() as RawFd,
            None => -1,
        }
    }
    unsafe fn from_c(fd: RawFd) -> Self {
        if fd < 0 {
            None
        } else {
            Some(unsafe { BorrowedFd::borrow_raw(fd as _) })
        }
    }
}

impl<T: FfiHandle + 'static> FfiType for &T {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
    const IS_HANDLE: bool = true;
    fn into_c(self) -> *mut core::ffi::c_void {
        unsafe { FfiHandle::as_handle(self) }
    }
    unsafe fn from_c(_: *mut core::ffi::c_void) -> Self {
        unimplemented!("&T from_c")
    }
}
impl<T: FfiHandle + 'static> FfiType for &mut T {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
    const IS_HANDLE: bool = true;
    fn into_c(self) -> *mut core::ffi::c_void {
        unsafe { FfiHandle::as_handle(self) }
    }
    unsafe fn from_c(_: *mut core::ffi::c_void) -> Self {
        unimplemented!("&mut T from_c")
    }
}

/// Borrowed slice of handles returned from FFI methods.
/// Elements are borrowed — do NOT destroy them individually.
/// Derefs to `&[T]`, so indexing, iteration, `len()`, etc. all work.
/// Dropping this type frees the backing array.
pub struct ForeignSlice<T> {
    raw: ffier::FfierObjectArray,
    elements: Box<[core::mem::ManuallyDrop<T>]>,
}

impl<T: FfiHandle> ForeignSlice<T> {
    fn from_raw(raw: ffier::FfierObjectArray) -> Self {
        let elements: Box<[core::mem::ManuallyDrop<T>]> = (0..raw.len)
            .map(|i| {
                core::mem::ManuallyDrop::new(unsafe {
                    T::__from_raw(ffier::ffier_object_array_get(raw, i))
                })
            })
            .collect();
        Self { raw, elements }
    }
}

impl<T> core::ops::Deref for ForeignSlice<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: ManuallyDrop<T> is #[repr(transparent)] over T.
        unsafe { core::mem::transmute::<&[core::mem::ManuallyDrop<T>], &[T]>(&self.elements) }
    }
}

impl<T> Drop for ForeignSlice<T> {
    fn drop(&mut self) {
        unsafe { ffier::ffier_object_array_free(self.raw) };
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inner {
    pub value: u32,
}
#[doc(hidden)]
#[repr(C)]
pub struct __FfierCReprInner {
    pub value: <u32 as FfiType>::CRepr,
}
impl FfiType for Inner {
    type CRepr = __FfierCReprInner;
    const C_TYPE_NAME: &'static str = "SubtestInner";
    fn into_c(self) -> Self::CRepr {
        __FfierCReprInner {
            value: <u32 as FfiType>::into_c(self.value),
        }
    }
    unsafe fn from_c(repr: Self::CRepr) -> Self {
        Self {
            value: unsafe { <u32 as FfiType>::from_c(repr.value) },
        }
    }
}
unsafe impl BorrowFFiType for Inner {}
const _: () = {
    assert!(core::mem::size_of::<Inner>() == core::mem::size_of::<__FfierCReprInner>());
    assert!(core::mem::align_of::<Inner>() == core::mem::align_of::<__FfierCReprInner>());
    assert!(core::mem::offset_of!(Inner, value) == core::mem::offset_of!(__FfierCReprInner, value));
};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outer {
    pub inner: Inner,
    pub crate_inner: Inner,
    pub self_inner: Inner,
}
#[doc(hidden)]
#[repr(C)]
pub struct __FfierCReprOuter {
    pub inner: <Inner as FfiType>::CRepr,
    pub crate_inner: <Inner as FfiType>::CRepr,
    pub self_inner: <Inner as FfiType>::CRepr,
}
impl FfiType for Outer {
    type CRepr = __FfierCReprOuter;
    const C_TYPE_NAME: &'static str = "SubtestOuter";
    fn into_c(self) -> Self::CRepr {
        __FfierCReprOuter {
            inner: <Inner as FfiType>::into_c(self.inner),
            crate_inner: <Inner as FfiType>::into_c(self.crate_inner),
            self_inner: <Inner as FfiType>::into_c(self.self_inner),
        }
    }
    unsafe fn from_c(repr: Self::CRepr) -> Self {
        Self {
            inner: unsafe { <Inner as FfiType>::from_c(repr.inner) },
            crate_inner: unsafe { <Inner as FfiType>::from_c(repr.crate_inner) },
            self_inner: unsafe { <Inner as FfiType>::from_c(repr.self_inner) },
        }
    }
}
unsafe impl BorrowFFiType for Outer {}
const _: () = {
    assert!(core::mem::size_of::<Outer>() == core::mem::size_of::<__FfierCReprOuter>());
    assert!(core::mem::align_of::<Outer>() == core::mem::align_of::<__FfierCReprOuter>());
    assert!(core::mem::offset_of!(Outer, inner) == core::mem::offset_of!(__FfierCReprOuter, inner));
    assert!(
        core::mem::offset_of!(Outer, crate_inner)
            == core::mem::offset_of!(__FfierCReprOuter, crate_inner)
    );
    assert!(
        core::mem::offset_of!(Outer, self_inner)
            == core::mem::offset_of!(__FfierCReprOuter, self_inner)
    );
};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScalarValues {
    pub i8_value: i8,
    pub i16_value: i16,
    pub i32_value: i32,
    pub i64_value: i64,
    pub u8_value: u8,
    pub u16_value: u16,
    pub u32_value: u32,
    pub u64_value: u64,
    pub offset: isize,
    pub size: usize,
    pub enabled: bool,
    pub f32_value: f32,
    pub f64_value: f64,
}
#[doc(hidden)]
#[repr(C)]
pub struct __FfierCReprScalarValues {
    pub i8_value: <i8 as FfiType>::CRepr,
    pub i16_value: <i16 as FfiType>::CRepr,
    pub i32_value: <i32 as FfiType>::CRepr,
    pub i64_value: <i64 as FfiType>::CRepr,
    pub u8_value: <u8 as FfiType>::CRepr,
    pub u16_value: <u16 as FfiType>::CRepr,
    pub u32_value: <u32 as FfiType>::CRepr,
    pub u64_value: <u64 as FfiType>::CRepr,
    pub offset: <isize as FfiType>::CRepr,
    pub size: <usize as FfiType>::CRepr,
    pub enabled: <bool as FfiType>::CRepr,
    pub f32_value: <f32 as FfiType>::CRepr,
    pub f64_value: <f64 as FfiType>::CRepr,
}
impl FfiType for ScalarValues {
    type CRepr = __FfierCReprScalarValues;
    const C_TYPE_NAME: &'static str = "SubtestScalarValues";
    fn into_c(self) -> Self::CRepr {
        __FfierCReprScalarValues {
            i8_value: <i8 as FfiType>::into_c(self.i8_value),
            i16_value: <i16 as FfiType>::into_c(self.i16_value),
            i32_value: <i32 as FfiType>::into_c(self.i32_value),
            i64_value: <i64 as FfiType>::into_c(self.i64_value),
            u8_value: <u8 as FfiType>::into_c(self.u8_value),
            u16_value: <u16 as FfiType>::into_c(self.u16_value),
            u32_value: <u32 as FfiType>::into_c(self.u32_value),
            u64_value: <u64 as FfiType>::into_c(self.u64_value),
            offset: <isize as FfiType>::into_c(self.offset),
            size: <usize as FfiType>::into_c(self.size),
            enabled: <bool as FfiType>::into_c(self.enabled),
            f32_value: <f32 as FfiType>::into_c(self.f32_value),
            f64_value: <f64 as FfiType>::into_c(self.f64_value),
        }
    }
    unsafe fn from_c(repr: Self::CRepr) -> Self {
        Self {
            i8_value: unsafe { <i8 as FfiType>::from_c(repr.i8_value) },
            i16_value: unsafe { <i16 as FfiType>::from_c(repr.i16_value) },
            i32_value: unsafe { <i32 as FfiType>::from_c(repr.i32_value) },
            i64_value: unsafe { <i64 as FfiType>::from_c(repr.i64_value) },
            u8_value: unsafe { <u8 as FfiType>::from_c(repr.u8_value) },
            u16_value: unsafe { <u16 as FfiType>::from_c(repr.u16_value) },
            u32_value: unsafe { <u32 as FfiType>::from_c(repr.u32_value) },
            u64_value: unsafe { <u64 as FfiType>::from_c(repr.u64_value) },
            offset: unsafe { <isize as FfiType>::from_c(repr.offset) },
            size: unsafe { <usize as FfiType>::from_c(repr.size) },
            enabled: unsafe { <bool as FfiType>::from_c(repr.enabled) },
            f32_value: unsafe { <f32 as FfiType>::from_c(repr.f32_value) },
            f64_value: unsafe { <f64 as FfiType>::from_c(repr.f64_value) },
        }
    }
}
unsafe impl BorrowFFiType for ScalarValues {}
const _: () = {
    assert!(
        core::mem::size_of::<ScalarValues>() == core::mem::size_of::<__FfierCReprScalarValues>()
    );
    assert!(
        core::mem::align_of::<ScalarValues>() == core::mem::align_of::<__FfierCReprScalarValues>()
    );
    assert!(
        core::mem::offset_of!(ScalarValues, i8_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, i8_value)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, i16_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, i16_value)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, i32_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, i32_value)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, i64_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, i64_value)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, u8_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, u8_value)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, u16_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, u16_value)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, u32_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, u32_value)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, u64_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, u64_value)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, offset)
            == core::mem::offset_of!(__FfierCReprScalarValues, offset)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, size)
            == core::mem::offset_of!(__FfierCReprScalarValues, size)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, enabled)
            == core::mem::offset_of!(__FfierCReprScalarValues, enabled)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, f32_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, f32_value)
    );
    assert!(
        core::mem::offset_of!(ScalarValues, f64_value)
            == core::mem::offset_of!(__FfierCReprScalarValues, f64_value)
    );
};

#[cfg(feature = "extra")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfigValue {
    pub base: u32,
}
#[cfg(feature = "extra")]
#[doc(hidden)]
#[repr(C)]
pub struct __FfierCReprConfigValue {
    pub base: <u32 as FfiType>::CRepr,
}
#[cfg(feature = "extra")]
impl FfiType for ConfigValue {
    type CRepr = __FfierCReprConfigValue;
    const C_TYPE_NAME: &'static str = "SubtestConfigValue";
    fn into_c(self) -> Self::CRepr {
        __FfierCReprConfigValue {
            base: <u32 as FfiType>::into_c(self.base),
        }
    }
    unsafe fn from_c(repr: Self::CRepr) -> Self {
        Self {
            base: unsafe { <u32 as FfiType>::from_c(repr.base) },
        }
    }
}
#[cfg(feature = "extra")]
unsafe impl BorrowFFiType for ConfigValue {}
#[cfg(feature = "extra")]
const _: () = {
    assert!(core::mem::size_of::<ConfigValue>() == core::mem::size_of::<__FfierCReprConfigValue>());
    assert!(
        core::mem::align_of::<ConfigValue>() == core::mem::align_of::<__FfierCReprConfigValue>()
    );
    assert!(
        core::mem::offset_of!(ConfigValue, base)
            == core::mem::offset_of!(__FfierCReprConfigValue, base)
    );
};

unsafe extern "C" {
    fn subtest_error_new(r: ffier::FfierResult) -> *mut core::ffi::c_void;
    fn subtest_error_field(
        handle: *const core::ffi::c_void,
        index: usize,
        out_buf: *mut core::ffi::c_void,
        buf_size: usize,
    );
}

/// Shared access to a handle field, tied to the owning error's lifetime.
pub struct ErrorFieldRef<'a, T> {
    value: T,
    lifetime: core::marker::PhantomData<&'a T>,
}
impl<T> core::ops::Deref for ErrorFieldRef<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

unsafe extern "C" {
    fn subtest_sub_error_payload_new(
        field_0: <i32 as FfiType>::CRepr,
        field_1: <Box<str> as FfiType>::CRepr,
        field_2: <Counter as FfiType>::CRepr,
        field_3: <Inner as FfiType>::CRepr,
    ) -> *mut core::ffi::c_void;
}

pub struct SubErrorErrorHandle(*mut core::ffi::c_void);
impl SubErrorErrorHandle {
    fn handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}
impl Drop for SubErrorErrorHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { subtest_error_destroy(self.0) }
        }
    }
}
impl std::fmt::Debug for SubErrorErrorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ErrorHandle({:?})", self.0)
    }
}

pub struct SubErrorPayloadData(SubErrorErrorHandle);
impl SubErrorPayloadData {
    pub fn new(field_0: i32, field_1: Box<str>, field_2: Counter, field_3: Inner) -> Self {
        let raw = unsafe {
            subtest_sub_error_payload_new(
                <i32 as FfiType>::into_c(field_0),
                <&str as FfiType>::into_c(field_1.as_ref()),
                <Counter as FfiType>::into_c(field_2),
                <Inner as FfiType>::into_c(field_3),
            )
        };
        assert!(
            !raw.is_null(),
            "library could not construct SubError::Payload"
        );
        Self(SubErrorErrorHandle(raw))
    }
    pub fn field_0(&self) -> i32 {
        let mut __buf = std::mem::MaybeUninit::<<i32 as FfiType>::CRepr>::uninit();
        unsafe {
            subtest_error_field(
                self.0.handle() as *const core::ffi::c_void,
                0,
                __buf.as_mut_ptr() as *mut core::ffi::c_void,
                core::mem::size_of::<<i32 as FfiType>::CRepr>(),
            )
        };
        unsafe { <i32 as FfiType>::from_c(__buf.assume_init()) }
    }
    pub fn field_1(&self) -> &str {
        let mut __buf = std::mem::MaybeUninit::<<&'static str as FfiType>::CRepr>::uninit();
        unsafe {
            subtest_error_field(
                self.0.handle() as *const core::ffi::c_void,
                1,
                __buf.as_mut_ptr() as *mut core::ffi::c_void,
                core::mem::size_of::<<&'static str as FfiType>::CRepr>(),
            )
        };
        unsafe { <&str as FfiType>::from_c(__buf.assume_init()) }
    }
    pub fn field_2(&self) -> ErrorFieldRef<'_, Counter> {
        let mut __buf = std::mem::MaybeUninit::<*mut core::ffi::c_void>::uninit();
        unsafe {
            subtest_error_field(
                self.0.handle() as *const core::ffi::c_void,
                2,
                __buf.as_mut_ptr() as *mut core::ffi::c_void,
                core::mem::size_of::<*mut core::ffi::c_void>(),
            )
        };
        ErrorFieldRef {
            value: unsafe { <Counter as FfiType>::from_c(__buf.assume_init()) },
            lifetime: core::marker::PhantomData,
        }
    }
    pub fn field_3(&self) -> Inner {
        let mut __buf = std::mem::MaybeUninit::<<Inner as FfiType>::CRepr>::uninit();
        unsafe {
            subtest_error_field(
                self.0.handle() as *const core::ffi::c_void,
                3,
                __buf.as_mut_ptr() as *mut core::ffi::c_void,
                core::mem::size_of::<<Inner as FfiType>::CRepr>(),
            )
        };
        unsafe { <Inner as FfiType>::from_c(__buf.assume_init()) }
    }
}
impl std::fmt::Debug for SubErrorPayloadData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Payload(...)")
    }
}

#[derive(Debug)]
pub enum SubError {
    Oops,
    BadThing(),
    Payload(SubErrorPayloadData),
}

impl SubError {
    fn fieldless_handle(code: u32) -> SubErrorErrorHandle {
        let raw = unsafe { subtest_error_new(ffier::ffier_result(33554433u32, code)) };
        assert!(
            !raw.is_null(),
            "library cannot construct SubError error code {code}"
        );
        SubErrorErrorHandle(raw)
    }
    /// Import an owned native error handle.
    ///
    /// # Safety
    /// The handle must be valid, non-null, and uniquely owned by the caller.
    /// Its native error type and variant must match the type tag and code in `r`.
    pub unsafe fn from_ffi(r: ffier::FfierResult, err_handle: *mut core::ffi::c_void) -> Self {
        let code = ffier::ffier_result_code(r);
        let handle = SubErrorErrorHandle(err_handle);
        match code {
            1u32 => {
                drop(handle);
                Self::Oops
            }
            2u32 => {
                drop(handle);
                Self::BadThing()
            }
            3u32 => Self::Payload(SubErrorPayloadData(handle)),
            other => panic!("unknown {} error code {}", "SubError", other),
        }
    }
    #[allow(dead_code)]
    fn into_ffi(self) -> (ffier::FfierResult, *mut core::ffi::c_void) {
        let (code, handle) = match self {
            Self::Oops => (1u32, Self::fieldless_handle(1u32)),
            Self::BadThing() => (2u32, Self::fieldless_handle(2u32)),
            Self::Payload(data) => (3u32, data.0),
        };
        let raw = handle.handle();
        core::mem::forget(handle);
        (ffier::ffier_result(33554433u32, code), raw)
    }
    fn message_handle(&self) -> (*mut core::ffi::c_void, Option<SubErrorErrorHandle>) {
        match self {
            Self::Oops => {
                let handle = Self::fieldless_handle(1u32);
                (handle.handle(), Some(handle))
            }
            Self::BadThing() => {
                let handle = Self::fieldless_handle(2u32);
                (handle.handle(), Some(handle))
            }
            Self::Payload(d) => (d.0.handle(), None),
        }
    }
}

impl std::fmt::Display for SubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct FmtWriter(*mut core::ffi::c_void);
        impl PushStr for FmtWriter {
            fn push(&mut self, s: &str) -> bool {
                unsafe {
                    (&mut *(self.0 as *mut std::fmt::Formatter<'_>))
                        .write_str(s)
                        .is_ok()
                }
            }
        }
        let mut __writer = FmtWriter(f as *mut std::fmt::Formatter<'_> as *mut core::ffi::c_void);
        let __vtable: &'static PushStrVtable = __ffier_vtable_PushStr::<FmtWriter>();
        let mut __temp = ffier::FfierHandle {
            type_tag: 33554436u32,
            metadata: 0,
            value: ffier::VtableHandle {
                vtable_ptr: __vtable as *const PushStrVtable as *const core::ffi::c_void,
                user_data: &mut __writer as *mut FmtWriter as *const core::ffi::c_void,
                vtable_size: core::mem::size_of::<PushStrVtable>() as u16,
            },
        };
        let __writer_handle =
            &mut __temp as *mut ffier::FfierHandle<ffier::VtableHandle> as *mut core::ffi::c_void;
        let (__error_handle, __temporary) = self.message_handle();
        unsafe { subtest_error_message(__error_handle, __writer_handle) };
        Ok(())
    }
}

impl std::error::Error for SubError {}

unsafe extern "C" {
    pub fn subtest_counter_destroy(handle: *mut core::ffi::c_void);
    pub fn subtest_counter_new() -> <Counter as FfiType>::CRepr;
    pub fn subtest_counter_increment(handle: *mut core::ffi::c_void);
    pub fn subtest_counter_get(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

pub struct Counter(*mut core::ffi::c_void);

impl Counter {
    #[doc(hidden)]
    /// # Safety
    /// Handle must be valid, uniquely owned, and valid for every lifetime of Self.
    pub unsafe fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Counter {
    const C_HANDLE_NAME: &'static str = "SubtestCounter";
    const TYPE_TAG: u32 = 33554434u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
    unsafe fn __from_raw(handle: *mut core::ffi::c_void) -> Self {
        Self(handle)
    }
}

impl FfiType for Counter {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Counter";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {
        unsafe { Self::__from_raw(repr) }
    }
}

impl std::fmt::Debug for Counter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Counter").field(&self.0).finish()
    }
}

impl Counter {
    pub fn new() -> Counter {
        let __raw = unsafe { subtest_counter_new() };
        unsafe { <Counter as FfiType>::from_c(__raw) }
    }
    pub fn increment(&mut self) {
        unsafe { subtest_counter_increment(self.0) }
    }
    pub fn get(&self) -> i32 {
        let __raw = unsafe { subtest_counter_get(self.0) };
        unsafe { <i32 as FfiType>::from_c(__raw) }
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        unsafe { subtest_counter_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn subtest_doubler_destroy(handle: *mut core::ffi::c_void);
    pub fn subtest_doubler_new() -> <Doubler as FfiType>::CRepr;
    pub fn subtest_doubler_double(
        handle: *mut core::ffi::c_void,
        counter: <&'static Counter as FfiType>::CRepr,
    ) -> <i32 as FfiType>::CRepr;
}

pub struct Doubler(*mut core::ffi::c_void);

impl Doubler {
    #[doc(hidden)]
    /// # Safety
    /// Handle must be valid, uniquely owned, and valid for every lifetime of Self.
    pub unsafe fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Doubler {
    const C_HANDLE_NAME: &'static str = "SubtestDoubler";
    const TYPE_TAG: u32 = 33554435u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
    unsafe fn __from_raw(handle: *mut core::ffi::c_void) -> Self {
        Self(handle)
    }
}

impl FfiType for Doubler {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Doubler";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {
        unsafe { Self::__from_raw(repr) }
    }
}

impl std::fmt::Debug for Doubler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Doubler").field(&self.0).finish()
    }
}

impl Doubler {
    pub fn new() -> Doubler {
        let __raw = unsafe { subtest_doubler_new() };
        unsafe { <Doubler as FfiType>::from_c(__raw) }
    }
    pub fn double(&self, counter: &Counter) -> i32 {
        let __raw = unsafe { subtest_doubler_double(self.0, FfiHandle::as_handle(counter)) };
        unsafe { <i32 as FfiType>::from_c(__raw) }
    }
}

impl Default for Doubler {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Doubler {
    fn drop(&mut self) {
        unsafe { subtest_doubler_destroy(self.0) }
    }
}

#[doc(hidden)]
pub enum __FfierCallbackTagPushStr {}
pub trait PushStr {
    fn push(&mut self, s: &str) -> bool;
    /// # Safety
    /// Callee must destroy this handle exactly once before any borrowed callback data expires.
    #[doc(hidden)]
    unsafe fn __into_raw_handle(self) -> *mut core::ffi::c_void
    where
        Self: Sized,
    {
        unsafe { self.__into_callback_handle().into_raw() }
    }
    #[doc(hidden)]
    fn __into_callback_handle<'callback>(
        self,
    ) -> ffier::CallbackHandle<'callback, __FfierCallbackTagPushStr>
    where
        Self: Sized + 'callback,
    {
        unsafe fn destroy(raw: *mut core::ffi::c_void) {
            let handle = unsafe { ffier::ffier_handle_consume::<ffier::VtableHandle>(raw) };
            let drop_callback: Option<unsafe extern "C" fn(*mut core::ffi::c_void)> =
                unsafe { handle.field_or_none(core::mem::offset_of!(PushStrVtable, drop)) };
            if let Some(drop_callback) = drop_callback {
                unsafe { drop_callback(handle.user_data as *mut core::ffi::c_void) }
            }
        }
        let value = ffier::VtableHandle {
            vtable_ptr: __ffier_vtable_PushStr::<Self>() as *const PushStrVtable
                as *const core::ffi::c_void,
            user_data: Box::into_raw(Box::new(self)) as *const core::ffi::c_void,
            vtable_size: core::mem::size_of::<PushStrVtable>()
                .try_into()
                .expect("vtable_size exceeds u16::MAX"),
        };
        unsafe {
            ffier::CallbackHandle::from_raw(ffier::ffier_handle_new(33554436, value), destroy)
        }
    }
    #[doc(hidden)]
    fn __borrow_callback_handle(
        &self,
    ) -> ffier::CallbackHandle<'_, __FfierCallbackTagPushStr, ffier::BorrowedCallback<false>>
    where
        Self: Sized,
    {
        let value = ffier::VtableHandle {
            vtable_ptr: __ffier_vtable_PushStr::<Self>() as *const PushStrVtable
                as *const core::ffi::c_void,
            user_data: self as *const Self as *const core::ffi::c_void,
            vtable_size: core::mem::size_of::<PushStrVtable>()
                .try_into()
                .expect("vtable_size exceeds u16::MAX"),
        };
        unsafe {
            ffier::CallbackHandle::from_raw(
                ffier::ffier_handle_new(33554436, value),
                ffier::ffier_handle_drop::<ffier::VtableHandle>,
            )
        }
    }
    #[doc(hidden)]
    fn __borrow_callback_handle_mut(
        &mut self,
    ) -> ffier::CallbackHandle<'_, __FfierCallbackTagPushStr, ffier::BorrowedCallback<true>>
    where
        Self: Sized,
    {
        let value = ffier::VtableHandle {
            vtable_ptr: __ffier_vtable_PushStr::<Self>() as *const PushStrVtable
                as *const core::ffi::c_void,
            user_data: self as *mut Self as *const core::ffi::c_void,
            vtable_size: core::mem::size_of::<PushStrVtable>()
                .try_into()
                .expect("vtable_size exceeds u16::MAX"),
        };
        unsafe {
            ffier::CallbackHandle::from_raw(
                ffier::ffier_handle_new(33554436, value),
                ffier::ffier_handle_drop::<ffier::VtableHandle>,
            )
        }
    }
}

#[repr(C)]
pub struct PushStrVtable {
    pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
    pub push: Option<
        unsafe extern "C" fn(
            *mut core::ffi::c_void,
            <&'static str as FfiType>::CRepr,
        ) -> <bool as FfiType>::CRepr,
    >,
}

#[allow(non_snake_case)]
fn __ffier_vtable_PushStr<__T: PushStr>() -> &'static PushStrVtable {
    &PushStrVtable {
        drop: Some({
            unsafe extern "C" fn __drop_trampoline<__T>(__ud: *mut core::ffi::c_void) {
                unsafe { drop(Box::from_raw(__ud as *mut __T)) };
            }
            __drop_trampoline::<__T>
        }),
        push: Some({
            unsafe extern "C" fn __trampoline<__T: PushStr>(
                __ud: *mut core::ffi::c_void,
                s: <&'static str as FfiType>::CRepr,
            ) -> <bool as FfiType>::CRepr {
                let __val = unsafe { &mut *(__ud as *mut __T) };
                let __result = __val.push(unsafe { <&str as FfiType>::from_c(s) });
                <bool as FfiType>::into_c(__result)
            }
            __trampoline::<__T>
        }),
    }
}
unsafe extern "C" {
    pub fn subtest_push_str_destroy(handle: *mut core::ffi::c_void);
}

pub struct VtablePushStr<'callback>(
    *mut core::ffi::c_void,
    core::marker::PhantomData<&'callback mut ()>,
);

impl VtablePushStr<'_> {
    #[doc(hidden)]
    /// # Safety
    /// Callee must destroy this handle exactly once within its callback lifetime.
    pub unsafe fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl Drop for VtablePushStr<'_> {
    fn drop(&mut self) {
        unsafe { subtest_push_str_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn subtest_error_code(handle: *mut core::ffi::c_void) -> <u32 as FfiType>::CRepr;
    pub fn subtest_error_message(handle: *mut core::ffi::c_void, writer: *mut core::ffi::c_void);
    pub fn subtest_error_result(handle: *mut core::ffi::c_void) -> <u64 as FfiType>::CRepr;
    pub fn subtest_error_destroy(handle: *mut core::ffi::c_void);
}

#[doc(hidden)]
pub enum __FfierCallbackTagFdSource {}
pub trait FdSource: ::core::marker::Send {
    fn read_notify_fd(&self) -> Result<BorrowedFd<'_>, SubError>;
    fn clone_fd(&self, fd: BorrowedFd<'_>) -> Result<OwnedFd, SubError>;
    fn preserve_fd(&self, fd: OwnedFd) -> OwnedFd;
    fn raw_fd(&self, fd: RawFd) -> RawFd;
    fn borrowed_fd<'a>(&self, fd: BorrowedFd<'a>) -> Option<BorrowedFd<'a>>;
    /// # Safety
    /// Callee must destroy this handle exactly once before any borrowed callback data expires.
    #[doc(hidden)]
    unsafe fn __into_raw_handle(self) -> *mut core::ffi::c_void
    where
        Self: Sized,
    {
        unsafe { self.__into_callback_handle().into_raw() }
    }
    #[doc(hidden)]
    fn __into_callback_handle<'callback>(
        self,
    ) -> ffier::CallbackHandle<
        'callback,
        __FfierCallbackTagFdSource,
        ffier::OwnedCallback,
        true,
        false,
    >
    where
        Self: Sized + 'callback,
    {
        unsafe fn destroy(raw: *mut core::ffi::c_void) {
            let handle = unsafe { ffier::ffier_handle_consume::<ffier::VtableHandle>(raw) };
            let drop_callback: Option<unsafe extern "C" fn(*mut core::ffi::c_void)> =
                unsafe { handle.field_or_none(core::mem::offset_of!(FdSourceVtable, drop)) };
            if let Some(drop_callback) = drop_callback {
                unsafe { drop_callback(handle.user_data as *mut core::ffi::c_void) }
            }
        }
        let value = ffier::VtableHandle {
            vtable_ptr: __ffier_vtable_FdSource::<Self>() as *const FdSourceVtable
                as *const core::ffi::c_void,
            user_data: Box::into_raw(Box::new(self)) as *const core::ffi::c_void,
            vtable_size: core::mem::size_of::<FdSourceVtable>()
                .try_into()
                .expect("vtable_size exceeds u16::MAX"),
        };
        unsafe {
            ffier::CallbackHandle::from_raw(ffier::ffier_handle_new(33554438, value), destroy)
        }
    }
    #[doc(hidden)]
    fn __borrow_callback_handle(
        &self,
    ) -> ffier::CallbackHandle<
        '_,
        __FfierCallbackTagFdSource,
        ffier::BorrowedCallback<false>,
        true,
        false,
    >
    where
        Self: Sized,
    {
        let value = ffier::VtableHandle {
            vtable_ptr: __ffier_vtable_FdSource::<Self>() as *const FdSourceVtable
                as *const core::ffi::c_void,
            user_data: self as *const Self as *const core::ffi::c_void,
            vtable_size: core::mem::size_of::<FdSourceVtable>()
                .try_into()
                .expect("vtable_size exceeds u16::MAX"),
        };
        unsafe {
            ffier::CallbackHandle::from_raw(
                ffier::ffier_handle_new(33554438, value),
                ffier::ffier_handle_drop::<ffier::VtableHandle>,
            )
        }
    }
    #[doc(hidden)]
    fn __borrow_callback_handle_mut(
        &mut self,
    ) -> ffier::CallbackHandle<
        '_,
        __FfierCallbackTagFdSource,
        ffier::BorrowedCallback<true>,
        true,
        false,
    >
    where
        Self: Sized,
    {
        let value = ffier::VtableHandle {
            vtable_ptr: __ffier_vtable_FdSource::<Self>() as *const FdSourceVtable
                as *const core::ffi::c_void,
            user_data: self as *mut Self as *const core::ffi::c_void,
            vtable_size: core::mem::size_of::<FdSourceVtable>()
                .try_into()
                .expect("vtable_size exceeds u16::MAX"),
        };
        unsafe {
            ffier::CallbackHandle::from_raw(
                ffier::ffier_handle_new(33554438, value),
                ffier::ffier_handle_drop::<ffier::VtableHandle>,
            )
        }
    }
}

#[repr(C)]
pub struct FdSourceVtable {
    pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
    pub read_notify_fd: Option<
        unsafe extern "C" fn(
            *mut core::ffi::c_void,
            *mut <BorrowedFd<'static> as FfiType>::CRepr,
            *mut *mut core::ffi::c_void,
        ) -> ffier::FfierResult,
    >,
    pub clone_fd: Option<
        unsafe extern "C" fn(
            *mut core::ffi::c_void,
            <BorrowedFd<'static> as FfiType>::CRepr,
            *mut <OwnedFd as FfiType>::CRepr,
            *mut *mut core::ffi::c_void,
        ) -> ffier::FfierResult,
    >,
    pub preserve_fd: Option<
        unsafe extern "C" fn(
            *mut core::ffi::c_void,
            <OwnedFd as FfiType>::CRepr,
        ) -> <OwnedFd as FfiType>::CRepr,
    >,
    pub raw_fd: Option<
        unsafe extern "C" fn(
            *mut core::ffi::c_void,
            <RawFd as FfiType>::CRepr,
        ) -> <RawFd as FfiType>::CRepr,
    >,
    pub borrowed_fd: Option<
        unsafe extern "C" fn(
            *mut core::ffi::c_void,
            <BorrowedFd<'static> as FfiType>::CRepr,
        ) -> <Option<BorrowedFd<'static>> as FfiType>::CRepr,
    >,
}

#[allow(non_snake_case)]
fn __ffier_vtable_FdSource<__T: FdSource>() -> &'static FdSourceVtable {
    &FdSourceVtable {
        drop: Some({
            unsafe extern "C" fn __drop_trampoline<__T>(__ud: *mut core::ffi::c_void) {
                unsafe { drop(Box::from_raw(__ud as *mut __T)) };
            }
            __drop_trampoline::<__T>
        }),
        read_notify_fd: Some({
            unsafe extern "C" fn __trampoline<__T: FdSource>(
                __ud: *mut core::ffi::c_void,
                result: *mut <BorrowedFd<'static> as FfiType>::CRepr,
                err_out: *mut *mut core::ffi::c_void,
            ) -> ffier::FfierResult {
                let __val = unsafe { &*(__ud as *const __T) };
                let __result = __val.read_notify_fd();
                match __result {
                    Ok(__ok) => {
                        unsafe { result.write(<BorrowedFd<'_> as FfiType>::into_c(__ok)) };
                        0 // FFIER_RESULT_SUCCESS
                    }
                    Err(__e) => {
                        let (__code, __handle) = __e.into_ffi();
                        unsafe { *err_out = __handle };
                        __code
                    }
                }
            }
            __trampoline::<__T>
        }),
        clone_fd: Some({
            unsafe extern "C" fn __trampoline<__T: FdSource>(
                __ud: *mut core::ffi::c_void,
                fd: <BorrowedFd<'static> as FfiType>::CRepr,
                result: *mut <OwnedFd as FfiType>::CRepr,
                err_out: *mut *mut core::ffi::c_void,
            ) -> ffier::FfierResult {
                let __val = unsafe { &*(__ud as *const __T) };
                let __result = __val.clone_fd(unsafe { <BorrowedFd<'_> as FfiType>::from_c(fd) });
                match __result {
                    Ok(__ok) => {
                        unsafe { result.write(<OwnedFd as FfiType>::into_c(__ok)) };
                        0 // FFIER_RESULT_SUCCESS
                    }
                    Err(__e) => {
                        let (__code, __handle) = __e.into_ffi();
                        unsafe { *err_out = __handle };
                        __code
                    }
                }
            }
            __trampoline::<__T>
        }),
        preserve_fd: Some({
            unsafe extern "C" fn __trampoline<__T: FdSource>(
                __ud: *mut core::ffi::c_void,
                fd: <OwnedFd as FfiType>::CRepr,
            ) -> <OwnedFd as FfiType>::CRepr {
                let __val = unsafe { &*(__ud as *const __T) };
                let __result = __val.preserve_fd(unsafe { <OwnedFd as FfiType>::from_c(fd) });
                <OwnedFd as FfiType>::into_c(__result)
            }
            __trampoline::<__T>
        }),
        raw_fd: Some({
            unsafe extern "C" fn __trampoline<__T: FdSource>(
                __ud: *mut core::ffi::c_void,
                fd: <RawFd as FfiType>::CRepr,
            ) -> <RawFd as FfiType>::CRepr {
                let __val = unsafe { &*(__ud as *const __T) };
                let __result = __val.raw_fd(unsafe { <RawFd as FfiType>::from_c(fd) });
                <RawFd as FfiType>::into_c(__result)
            }
            __trampoline::<__T>
        }),
        borrowed_fd: Some({
            #[allow(clippy::extra_unused_lifetimes)]
            unsafe extern "C" fn __trampoline<'a, __T: FdSource>(
                __ud: *mut core::ffi::c_void,
                fd: <BorrowedFd<'static> as FfiType>::CRepr,
            ) -> <Option<BorrowedFd<'static>> as FfiType>::CRepr {
                let __val = unsafe { &*(__ud as *const __T) };
                let __result =
                    __val.borrowed_fd(unsafe { <BorrowedFd<'a> as FfiType>::from_c(fd) });
                <Option<BorrowedFd<'a>> as FfiType>::into_c(__result)
            }
            __trampoline::<__T>
        }),
    }
}
unsafe extern "C" {
    pub fn subtest_fd_source_destroy(handle: *mut core::ffi::c_void);
}

pub struct VtableFdSource<'callback>(
    *mut core::ffi::c_void,
    core::marker::PhantomData<&'callback mut ()>,
);
// SAFETY: library verifies this guarantee; unsafe imports must preserve it.
unsafe impl<'callback> ::core::marker::Send for VtableFdSource<'callback> {}

impl VtableFdSource<'_> {
    #[doc(hidden)]
    /// # Safety
    /// Callee must destroy this handle exactly once within its callback lifetime.
    pub unsafe fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl Drop for VtableFdSource<'_> {
    fn drop(&mut self) {
        unsafe { subtest_fd_source_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn subtest_probe_fd_source(
        source: *mut core::ffi::c_void,
        result: *mut <bool as FfiType>::CRepr,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
}

pub fn probe_fd_source(source: &mut impl FdSource) -> Result<bool, SubError> {
    let __ffi_source = source.__borrow_callback_handle_mut();
    let mut __out = std::mem::MaybeUninit::uninit();
    let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
    let __r = unsafe {
        subtest_probe_fd_source(
            __ffi_source.as_raw(),
            __out.as_mut_ptr(),
            &mut __err as *mut *mut core::ffi::c_void,
        )
    };
    if __r == 0 {
        Ok(unsafe { <bool as FfiType>::from_c(__out.assume_init()) })
    } else {
        Err(unsafe { SubError::from_ffi(__r, __err) })
    }
}

unsafe extern "C" {
    pub fn subtest_echo_scalars(
        values: <ScalarValues as FfiType>::CRepr,
    ) -> <ScalarValues as FfiType>::CRepr;
}

pub fn echo_scalars(values: ScalarValues) -> ScalarValues {
    let __raw = unsafe { subtest_echo_scalars(<ScalarValues as FfiType>::into_c(values)) };
    unsafe { <ScalarValues as FfiType>::from_c(__raw) }
}
