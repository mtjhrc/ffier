#[allow(unused_imports)]
use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

/// Marker trait for types exported as opaque C handles.
pub trait FfiHandle {
    const C_HANDLE_NAME: &'static str;
    const TYPE_TAG: u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void;
}

/// Maps Rust types to C-compatible representations.
pub trait FfiType {
    type CRepr;
    const C_TYPE_NAME: &'static str;
    const IS_HANDLE: bool = false;
    fn into_c(self) -> Self::CRepr;
    fn from_c(repr: Self::CRepr) -> Self;
}

macro_rules! impl_ffi_identity {
    ($($t:ty => $n:expr),* $(,)?) => { $(
        impl FfiType for $t {
            type CRepr = $t; const C_TYPE_NAME: &'static str = $n; const IS_HANDLE: bool = false;
            fn into_c(self) -> Self { self } fn from_c(r: Self) -> Self { r }
        }
    )* };
}
impl_ffi_identity! {
    i8 => "int8_t", i16 => "int16_t", i32 => "int32_t", i64 => "int64_t",
    u8 => "uint8_t", u16 => "uint16_t", u32 => "uint32_t", u64 => "uint64_t",
    isize => "ssize_t", usize => "size_t", bool => "bool",
}

impl FfiType for &str {
    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = "FfierStr"; const IS_HANDLE: bool = false;
    fn into_c(self) -> ffier::FfierBytes { unsafe { ffier::FfierBytes::from_str(self) } }
    fn from_c(repr: ffier::FfierBytes) -> Self { unsafe { let b = core::slice::from_raw_parts(repr.data, repr.len); core::str::from_utf8_unchecked(b) } }
}

impl FfiType for &[u8] {
    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = "FfierBytes"; const IS_HANDLE: bool = false;
    fn into_c(self) -> ffier::FfierBytes { unsafe { ffier::FfierBytes::from_bytes(self) } }
    fn from_c(repr: ffier::FfierBytes) -> Self { unsafe { if repr.data.is_null() { &[] } else { core::slice::from_raw_parts(repr.data, repr.len) } } }
}

impl FfiType for OwnedFd {
    type CRepr = i32; const C_TYPE_NAME: &'static str = "int"; const IS_HANDLE: bool = false;
    fn into_c(self) -> i32 { use std::os::unix::io::IntoRawFd; self.into_raw_fd() }
    fn from_c(fd: i32) -> Self { unsafe { OwnedFd::from_raw_fd(fd) } }
}
impl<'a> FfiType for BorrowedFd<'a> {
    type CRepr = i32; const C_TYPE_NAME: &'static str = "int"; const IS_HANDLE: bool = false;
    fn into_c(self) -> i32 { self.as_raw_fd() }
    fn from_c(fd: i32) -> Self { unsafe { BorrowedFd::borrow_raw(fd) } }
}

impl<T: FfiHandle + 'static> FfiType for &T {
    type CRepr = *mut core::ffi::c_void; const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME; const IS_HANDLE: bool = true;
    fn into_c(self) -> *mut core::ffi::c_void { unsafe { self.as_handle() } }
    fn from_c(_: *mut core::ffi::c_void) -> Self { unimplemented!("client-side &T from_c") }
}
impl<T: FfiHandle + 'static> FfiType for &mut T {
    type CRepr = *mut core::ffi::c_void; const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME; const IS_HANDLE: bool = true;
    fn into_c(self) -> *mut core::ffi::c_void { unsafe { self.as_handle() } }
    fn from_c(_: *mut core::ffi::c_void) -> Self { unimplemented!("client-side &mut T from_c") }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcError {
    DivisionByZero,
}

impl CalcError {
    pub fn from_ffi(r: ffier::FfierResult) -> Self {
        let code = ffier::ffier_result_code(r);
        match code {
            1u32 => Self::DivisionByZero,
            other => panic!("unknown {} error code {}", "CalcError", other),
        }
    }
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DivisionByZero => write!(f, "DivisionByZero"),
        }
    }
}

impl std::error::Error for CalcError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferError {
    WriteFailed,
}

impl BufferError {
    pub fn from_ffi(r: ffier::FfierResult) -> Self {
        let code = ffier::ffier_result_code(r);
        match code {
            1u32 => Self::WriteFailed,
            other => panic!("unknown {} error code {}", "BufferError", other),
        }
    }
}

impl std::fmt::Display for BufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WriteFailed => write!(f, "WriteFailed"),
        }
    }
}

impl std::error::Error for BufferError {}

unsafe extern "C" {
    pub fn mylib_calculator_destroy(handle: *mut core::ffi::c_void);
    pub fn mylib_calculator_new() -> <Calculator as FfiType>::CRepr;
    pub fn mylib_calculator_add(handle: *mut core::ffi::c_void, a: <i32 as FfiType>::CRepr, b: <i32 as FfiType>::CRepr) -> <i32 as FfiType>::CRepr;
    pub fn mylib_calculator_is_positive(handle: *mut core::ffi::c_void, value: <i32 as FfiType>::CRepr) -> <bool as FfiType>::CRepr;
    pub fn mylib_calculator_divide(handle: *mut core::ffi::c_void, a: <i32 as FfiType>::CRepr, b: <i32 as FfiType>::CRepr, result: *mut <i32 as FfiType>::CRepr, err_out: *mut *mut core::ffi::c_void) -> ffier::FfierResult;
}

pub struct Calculator(*mut core::ffi::c_void);

impl Calculator {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self { Self(ptr) }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void { let this = std::mem::ManuallyDrop::new(self); this.0 }
}

impl FfiHandle for Calculator {
    const C_HANDLE_NAME: &'static str = "Calculator";
    const TYPE_TAG: u32 = 1u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void { self.0 }
}

impl FfiType for Calculator {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Calculator";
    fn into_c(self) -> *mut core::ffi::c_void { self.__into_raw() }
    fn from_c(repr: *mut core::ffi::c_void) -> Self { Self::__from_raw(repr) }
}

impl std::fmt::Debug for Calculator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Calculator").field(&self.0).finish()
    }
}

impl Calculator {
    pub fn new() -> Calculator {
        let __raw = unsafe { mylib_calculator_new() };
        <Calculator as FfiType>::from_c(__raw)
    }
    #[doc = " Add two integers."]
    pub fn add(&self, a: i32, b: i32) -> i32 {
        let __raw = unsafe { mylib_calculator_add(self.0, <i32 as FfiType>::into_c(a), <i32 as FfiType>::into_c(b)) };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Check whether a value is strictly positive."]
    pub fn is_positive(&self, value: i32) -> bool {
        let __raw = unsafe { mylib_calculator_is_positive(self.0, <i32 as FfiType>::into_c(value)) };
        <bool as FfiType>::from_c(__raw)
    }
    #[doc = " Divide `a` by `b`, returning an error if `b` is zero."]
    pub fn divide(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        let mut __out = std::mem::MaybeUninit::uninit();
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r = unsafe { mylib_calculator_divide(self.0, <i32 as FfiType>::into_c(a), <i32 as FfiType>::into_c(b), __out.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) };
        if __r == 0 {
            Ok(<i32 as FfiType>::from_c(unsafe { __out.assume_init() }))
        } else {
            Err(CalcError::from_ffi(__r))
        }
    }
}

impl Drop for Calculator {
    fn drop(&mut self) {
        unsafe { mylib_calculator_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn mylib_text_buffer_destroy(handle: *mut core::ffi::c_void);
    pub fn mylib_text_buffer_new(output_fd: <OwnedFd as FfiType>::CRepr) -> <TextBuffer as FfiType>::CRepr;
    pub fn mylib_text_buffer_fd(handle: *mut core::ffi::c_void) -> <BorrowedFd<'static> as FfiType>::CRepr;
    pub fn mylib_text_buffer_write(handle: *mut core::ffi::c_void, text: <&'static str as FfiType>::CRepr);
    pub fn mylib_text_buffer_write_parts(handle: *mut core::ffi::c_void, parts: *const ffier::FfierBytes, parts_len: usize);
    pub fn mylib_text_buffer_contents(handle: *mut core::ffi::c_void) -> <&'static str as FfiType>::CRepr;
    pub fn mylib_text_buffer_as_bytes(handle: *mut core::ffi::c_void) -> <&'static [u8] as FfiType>::CRepr;
    pub fn mylib_text_buffer_flush(handle: *mut core::ffi::c_void, err_out: *mut *mut core::ffi::c_void) -> ffier::FfierResult;
    pub fn mylib_text_buffer_clear(handle: *mut core::ffi::c_void);
}

pub struct TextBuffer(*mut core::ffi::c_void);

impl TextBuffer {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self { Self(ptr) }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void { let this = std::mem::ManuallyDrop::new(self); this.0 }
}

impl FfiHandle for TextBuffer {
    const C_HANDLE_NAME: &'static str = "TextBuffer";
    const TYPE_TAG: u32 = 3u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void { self.0 }
}

impl FfiType for TextBuffer {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "TextBuffer";
    fn into_c(self) -> *mut core::ffi::c_void { self.__into_raw() }
    fn from_c(repr: *mut core::ffi::c_void) -> Self { Self::__from_raw(repr) }
}

impl std::fmt::Debug for TextBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TextBuffer").field(&self.0).finish()
    }
}

impl TextBuffer {
    #[doc = " Create a text buffer that writes to the given file descriptor."]
    pub fn new(output_fd: OwnedFd) -> TextBuffer {
        let __raw = unsafe { mylib_text_buffer_new(<OwnedFd as FfiType>::into_c(output_fd)) };
        <TextBuffer as FfiType>::from_c(__raw)
    }
    #[doc = " Get the output file descriptor."]
    pub fn fd(&self, ) -> BorrowedFd<'_> {
        let __raw = unsafe { mylib_text_buffer_fd(self.0) };
        <BorrowedFd<'_> as FfiType>::from_c(__raw)
    }
    #[doc = " Append text to the buffer."]
    pub fn write(&mut self, text: &str) {
        unsafe { mylib_text_buffer_write(self.0, <&str as FfiType>::into_c(text)) }
    }
    #[doc = " Append multiple strings to the buffer."]
    pub fn write_parts(&mut self, parts: &[&str]) {
        let __ffi_parts: Vec<ffier::FfierBytes> = parts.iter().map(|s| unsafe { ffier::FfierBytes::from_str(s) }).collect();
        unsafe { mylib_text_buffer_write_parts(self.0, __ffi_parts.as_ptr(), __ffi_parts.len()) }
    }
    #[doc = " Get the buffer contents."]
    pub fn contents(&self, ) -> &str {
        let __raw = unsafe { mylib_text_buffer_contents(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    #[doc = " Get the buffer contents as raw bytes."]
    pub fn as_bytes(&self, ) -> &[u8] {
        let __raw = unsafe { mylib_text_buffer_as_bytes(self.0) };
        <&[u8] as FfiType>::from_c(__raw)
    }
    #[doc = " Flush the buffer contents to the output file descriptor."]
    pub fn flush(&self, ) -> Result<(), BufferError> {
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r = unsafe { mylib_text_buffer_flush(self.0, &mut __err as *mut *mut core::ffi::c_void) };
        if __r == 0 { Ok(()) } else { Err(BufferError::from_ffi(__r)) }
    }
    pub fn clear(&mut self, ) {
        unsafe { mylib_text_buffer_clear(self.0) }
    }
}

impl Drop for TextBuffer {
    fn drop(&mut self) {
        unsafe { mylib_text_buffer_destroy(self.0) }
    }
}

