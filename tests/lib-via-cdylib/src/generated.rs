use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

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
    type CRepr = ffier::FfierBytes;
    const C_TYPE_NAME: &'static str = "FfierStr";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> ffier::FfierBytes {
        unsafe { ffier::FfierBytes::from_str(self) }
    }
    fn from_c(repr: ffier::FfierBytes) -> Self {
        unsafe {
            let b = core::slice::from_raw_parts(repr.data, repr.len);
            core::str::from_utf8_unchecked(b)
        }
    }
}

impl<'a> FfiType for Option<&'a str> {
    type CRepr = ffier::FfierBytes;
    const C_TYPE_NAME: &'static str = "FfierStr";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> ffier::FfierBytes {
        match self {
            Some(s) => unsafe { ffier::FfierBytes::from_str(s) },
            None => ffier::FfierBytes::EMPTY,
        }
    }
    fn from_c(repr: ffier::FfierBytes) -> Self {
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
            data: leaked.as_ptr(),
            len: leaked.len(),
        }
    }
    fn from_c(repr: ffier::FfierBytes) -> Self {
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
    fn from_c(repr: ffier::FfierBytes) -> Self {
        unsafe {
            if repr.data.is_null() {
                &[]
            } else {
                core::slice::from_raw_parts(repr.data, repr.len)
            }
        }
    }
}

impl FfiType for OwnedFd {
    type CRepr = RawFd;
    const C_TYPE_NAME: &'static str = "int";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> RawFd {
        use std::os::unix::io::IntoRawFd;
        self.into_raw_fd() as RawFd
    }
    fn from_c(fd: RawFd) -> Self {
        unsafe { OwnedFd::from_raw_fd(fd as _) }
    }
}

impl<'fd> FfiType for BorrowedFd<'fd> {
    type CRepr = RawFd;
    const C_TYPE_NAME: &'static str = "int";
    const IS_HANDLE: bool = false;
    fn into_c(self) -> RawFd {
        self.as_raw_fd() as RawFd
    }
    fn from_c(fd: RawFd) -> Self {
        unsafe { BorrowedFd::borrow_raw(fd as _) }
    }
}

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
    fn from_c(fd: RawFd) -> Self {
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
        unsafe { self.as_handle() }
    }
    fn from_c(_: *mut core::ffi::c_void) -> Self {
        unimplemented!("client-side &T from_c")
    }
}
impl<T: FfiHandle + 'static> FfiType for &mut T {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME;
    const IS_HANDLE: bool = true;
    fn into_c(self) -> *mut core::ffi::c_void {
        unsafe { self.as_handle() }
    }
    fn from_c(_: *mut core::ffi::c_void) -> Self {
        unimplemented!("client-side &mut T from_c")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum LogLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl FfiType for LogLevel {
    type CRepr = u32;
    const C_TYPE_NAME: &'static str = "LogLevel";
    fn into_c(self) -> u32 {
        self as u32
    }
    fn from_c(repr: u32) -> Self {
        match repr {
            0 => Self::Off,
            1 => Self::Error,
            2 => Self::Warn,
            3 => Self::Info,
            4 => Self::Debug,
            5 => Self::Trace,
            unknown => panic!("invalid LogLevel discriminant: {}", unknown),
        }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Permissions: u32 {
        const READ = 1;
        const WRITE = 2;
        const EXECUTE = 4;
        const DELETE = 8;
    }
}

impl FfiType for Permissions {
    type CRepr = u32;
    const C_TYPE_NAME: &'static str = "Permissions";
    fn into_c(self) -> u32 {
        self.bits()
    }
    fn from_c(repr: u32) -> Self {
        Self::from_bits_retain(repr)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestError {
    NotFound,
    CustomMessage,
    InvalidInput,
}

impl TestError {
    pub fn from_ffi(r: ffier::FfierResult) -> Self {
        let code = ffier::ffier_result_code(r);
        match code {
            1u32 => Self::NotFound,
            2u32 => Self::CustomMessage,
            3u32 => Self::InvalidInput,
            other => panic!("unknown {} error code {}", "TestError", other),
        }
    }
}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "NotFound(...)"),
            Self::CustomMessage => write!(f, "CustomMessage"),
            Self::InvalidInput => write!(f, "InvalidInput"),
        }
    }
}

impl std::error::Error for TestError {}

unsafe extern "C" {
    pub fn ft_widget_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_widget_new() -> <Widget as FfiType>::CRepr;
    pub fn ft_widget_with_name(
        name: <&'static str as FfiType>::CRepr,
    ) -> <Widget as FfiType>::CRepr;
    pub fn ft_widget_get_count(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
    pub fn ft_widget_set_count(handle: *mut core::ffi::c_void, n: <i32 as FfiType>::CRepr);
    pub fn ft_widget_with_count(handle: *mut core::ffi::c_void, n: <i32 as FfiType>::CRepr);
    pub fn ft_widget_name(handle: *mut core::ffi::c_void) -> <&'static str as FfiType>::CRepr;
    pub fn ft_widget_data(handle: *mut core::ffi::c_void) -> <&'static [u8] as FfiType>::CRepr;
    pub fn ft_widget_sum_bytes(
        handle: *mut core::ffi::c_void,
        data: <&'static [u8] as FfiType>::CRepr,
    ) -> <i32 as FfiType>::CRepr;
    pub fn ft_widget_echo(
        handle: *mut core::ffi::c_void,
        s: <&'static str as FfiType>::CRepr,
    ) -> <&'static str as FfiType>::CRepr;
    pub fn ft_widget_is_active(handle: *mut core::ffi::c_void) -> <bool as FfiType>::CRepr;
    pub fn ft_widget_negate(
        handle: *mut core::ffi::c_void,
        v: <i64 as FfiType>::CRepr,
    ) -> <i64 as FfiType>::CRepr;
    pub fn ft_widget_validate(
        handle: *mut core::ffi::c_void,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
    pub fn ft_widget_parse_count(
        handle: *mut core::ffi::c_void,
        s: <&'static str as FfiType>::CRepr,
        result: *mut <i32 as FfiType>::CRepr,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
    pub fn ft_widget_describe(
        handle: *mut core::ffi::c_void,
        code: <i32 as FfiType>::CRepr,
        result: *mut <&'static str as FfiType>::CRepr,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
    pub fn ft_widget_fail_always(
        handle: *mut core::ffi::c_void,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
    pub fn ft_widget_fail_with_value(
        handle: *mut core::ffi::c_void,
        result: *mut <i32 as FfiType>::CRepr,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
    pub fn ft_widget_set_tags(
        handle: *mut core::ffi::c_void,
        tags: *const ffier::FfierBytes,
        tags_len: usize,
    );
    pub fn ft_widget_tags_joined(
        handle: *mut core::ffi::c_void,
    ) -> <&'static str as FfiType>::CRepr;
    pub fn ft_widget_create_gadget(handle: *mut core::ffi::c_void) -> <Gadget as FfiType>::CRepr;
    pub fn ft_widget_try_create_gadget(
        handle: *mut core::ffi::c_void,
        ok: <bool as FfiType>::CRepr,
        err_out: *mut *mut core::ffi::c_void,
    ) -> *mut core::ffi::c_void;
    pub fn ft_widget_read_gadget(
        handle: *mut core::ffi::c_void,
        g: <&'static Gadget as FfiType>::CRepr,
    ) -> <i32 as FfiType>::CRepr;
    pub fn ft_widget_update_gadget(
        handle: *mut core::ffi::c_void,
        g: <&'static mut Gadget as FfiType>::CRepr,
        v: <i32 as FfiType>::CRepr,
    );
    pub fn ft_widget_set_name(
        handle: *mut core::ffi::c_void,
        name: <Option<&'static str> as FfiType>::CRepr,
    );
    pub fn ft_widget_owned_name(handle: *mut core::ffi::c_void) -> <Box<str> as FfiType>::CRepr;
    pub fn ft_widget_add_permission(
        handle: *mut core::ffi::c_void,
        base: <Permissions as FfiType>::CRepr,
        flag: <Permissions as FfiType>::CRepr,
    ) -> <Permissions as FfiType>::CRepr;
    pub fn ft_widget_consume(handle: *mut core::ffi::c_void);
    pub fn ft_widget_fd_number(
        handle: *mut core::ffi::c_void,
        fd: <BorrowedFd<'static> as FfiType>::CRepr,
    ) -> <i32 as FfiType>::CRepr;
    pub fn ft_widget_fd_number_optional(
        handle: *mut core::ffi::c_void,
        fd: <Option<BorrowedFd<'static>> as FfiType>::CRepr,
    ) -> <i32 as FfiType>::CRepr;
    pub fn ft_widget_maybe_fd(
        handle: *mut core::ffi::c_void,
        selector: <i32 as FfiType>::CRepr,
        result: *mut <Option<BorrowedFd<'static>> as FfiType>::CRepr,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
    pub fn ft_widget_dup_fd(
        handle: *mut core::ffi::c_void,
        fd: <BorrowedFd<'static> as FfiType>::CRepr,
    ) -> <OwnedFd as FfiType>::CRepr;
}

pub struct Widget(*mut core::ffi::c_void);

impl Widget {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Widget {
    const C_HANDLE_NAME: &'static str = "Widget";
    const TYPE_TAG: u32 = 2u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Widget {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Widget";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Widget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Widget").field(&self.0).finish()
    }
}

impl Widget {
    #[doc = " Create a new widget with default values."]
    pub fn new() -> Widget {
        let __raw = unsafe { ft_widget_new() };
        <Widget as FfiType>::from_c(__raw)
    }
    #[doc = " Create a widget with a given name."]
    pub fn with_name(name: &str) -> Widget {
        let __raw = unsafe { ft_widget_with_name(<&str as FfiType>::into_c(name)) };
        <Widget as FfiType>::from_c(__raw)
    }
    #[doc = " Get the current count."]
    pub fn get_count(&self) -> i32 {
        let __raw = unsafe { ft_widget_get_count(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Set the count."]
    pub fn set_count(&mut self, n: i32) {
        unsafe { ft_widget_set_count(self.0, <i32 as FfiType>::into_c(n)) }
    }
    #[doc = " Set count and return `&mut Self` for method chaining."]
    pub fn with_count(&mut self, n: i32) -> &mut Self {
        unsafe { ft_widget_with_count(self.0, <i32 as FfiType>::into_c(n)) };
        self
    }
    #[doc = " Get the widget name."]
    pub fn name(&self) -> &str {
        let __raw = unsafe { ft_widget_name(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    #[doc = " Get the raw name bytes."]
    pub fn data(&self) -> &[u8] {
        let __raw = unsafe { ft_widget_data(self.0) };
        <&[u8] as FfiType>::from_c(__raw)
    }
    #[doc = " Sum the bytes of a byte slice."]
    pub fn sum_bytes(&self, data: &[u8]) -> i32 {
        let __raw = unsafe { ft_widget_sum_bytes(self.0, <&[u8] as FfiType>::into_c(data)) };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Echo back the given string (zero-copy borrow passthrough)."]
    pub fn echo<'a>(&self, s: &'a str) -> &'a str {
        let __raw = unsafe { ft_widget_echo(self.0, <&'a str as FfiType>::into_c(s)) };
        <&'a str as FfiType>::from_c(__raw)
    }
    #[doc = " Check if the widget is active."]
    pub fn is_active(&self) -> bool {
        let __raw = unsafe { ft_widget_is_active(self.0) };
        <bool as FfiType>::from_c(__raw)
    }
    #[doc = " Negate a 64-bit integer."]
    pub fn negate(&self, v: i64) -> i64 {
        let __raw = unsafe { ft_widget_negate(self.0, <i64 as FfiType>::into_c(v)) };
        <i64 as FfiType>::from_c(__raw)
    }
    #[doc = " Validate internal state (always succeeds for default widget)."]
    pub fn validate(&self) -> Result<(), TestError> {
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r = unsafe { ft_widget_validate(self.0, &mut __err as *mut *mut core::ffi::c_void) };
        if __r == 0 {
            Ok(())
        } else {
            Err(TestError::from_ffi(__r))
        }
    }
    #[doc = " Parse a count value from the name length, returning error if name matches trigger."]
    #[doc = ""]
    #[doc = " # Arguments"]
    #[doc = ""]
    #[doc = " - `s`: the input string whose length becomes the count."]
    #[doc = ""]
    #[doc = " # Returns"]
    #[doc = ""]
    #[doc = " The count derived from the name length."]
    pub fn parse_count(&self, s: &str) -> Result<i32, TestError> {
        let mut __out = std::mem::MaybeUninit::uninit();
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r = unsafe {
            ft_widget_parse_count(
                self.0,
                <&str as FfiType>::into_c(s),
                __out.as_mut_ptr(),
                &mut __err as *mut *mut core::ffi::c_void,
            )
        };
        if __r == 0 {
            Ok(<i32 as FfiType>::from_c(unsafe { __out.assume_init() }))
        } else {
            Err(TestError::from_ffi(__r))
        }
    }
    #[doc = " Describe a code as a string."]
    #[doc = ""]
    #[doc = " # Arguments"]
    #[doc = ""]
    #[doc = " * `code` - the numeric code to look up."]
    pub fn describe(&self, code: i32) -> Result<&str, TestError> {
        let mut __out = std::mem::MaybeUninit::uninit();
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r = unsafe {
            ft_widget_describe(
                self.0,
                <i32 as FfiType>::into_c(code),
                __out.as_mut_ptr(),
                &mut __err as *mut *mut core::ffi::c_void,
            )
        };
        if __r == 0 {
            Ok(<&str as FfiType>::from_c(unsafe { __out.assume_init() }))
        } else {
            Err(TestError::from_ffi(__r))
        }
    }
    #[doc = " Always fails with an error."]
    pub fn fail_always(&self) -> Result<(), TestError> {
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r =
            unsafe { ft_widget_fail_always(self.0, &mut __err as *mut *mut core::ffi::c_void) };
        if __r == 0 {
            Ok(())
        } else {
            Err(TestError::from_ffi(__r))
        }
    }
    #[doc = " Always fails with an error (value variant)."]
    pub fn fail_with_value(&self) -> Result<i32, TestError> {
        let mut __out = std::mem::MaybeUninit::uninit();
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r = unsafe {
            ft_widget_fail_with_value(
                self.0,
                __out.as_mut_ptr(),
                &mut __err as *mut *mut core::ffi::c_void,
            )
        };
        if __r == 0 {
            Ok(<i32 as FfiType>::from_c(unsafe { __out.assume_init() }))
        } else {
            Err(TestError::from_ffi(__r))
        }
    }
    #[doc = " Set tags from a string slice."]
    pub fn set_tags(&mut self, tags: &[&str]) {
        let __ffi_tags: Vec<ffier::FfierBytes> = tags
            .iter()
            .map(|s| unsafe { ffier::FfierBytes::from_str(s) })
            .collect();
        unsafe { ft_widget_set_tags(self.0, __ffi_tags.as_ptr(), __ffi_tags.len()) }
    }
    #[doc = " Get joined tags."]
    pub fn tags_joined(&self) -> &str {
        let __raw = unsafe { ft_widget_tags_joined(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    #[doc = " Create a new gadget with the widget's count as initial value."]
    pub fn create_gadget(&self) -> Gadget {
        let __raw = unsafe { ft_widget_create_gadget(self.0) };
        <Gadget as FfiType>::from_c(__raw)
    }
    #[doc = " Try to create a gadget; fails if ok is false."]
    pub fn try_create_gadget(&self, ok: bool) -> Result<Gadget, TestError> {
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __raw = unsafe {
            ft_widget_try_create_gadget(
                self.0,
                <bool as FfiType>::into_c(ok),
                &mut __err as *mut *mut core::ffi::c_void,
            )
        };
        if !__raw.is_null() {
            Ok(<Gadget as FfiType>::from_c(__raw))
        } else {
            let __r = unsafe { ft_error_result(__err) };
            unsafe { ft_error_destroy(__err) };
            Err(TestError::from_ffi(__r))
        }
    }
    #[doc = " Read a gadget's value."]
    pub fn read_gadget(&self, g: &Gadget) -> i32 {
        let __raw = unsafe { ft_widget_read_gadget(self.0, FfiHandle::as_handle(g)) };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Update a gadget's value."]
    pub fn update_gadget(&self, g: &mut Gadget, v: i32) {
        unsafe {
            ft_widget_update_gadget(self.0, FfiHandle::as_handle(g), <i32 as FfiType>::into_c(v))
        }
    }
    #[doc = " Set the name, or reset to default if `None`."]
    pub fn set_name(&mut self, name: Option<&str>) {
        unsafe { ft_widget_set_name(self.0, <Option<&str> as FfiType>::into_c(name)) }
    }
    #[doc = " Get an owned copy of the name."]
    pub fn owned_name(&self) -> Box<str> {
        let __raw = unsafe { ft_widget_owned_name(self.0) };
        <Box<str> as FfiType>::from_c(__raw)
    }
    #[doc = " Add a permission flag to the widget's permissions and return the result."]
    pub fn add_permission(&self, base: Permissions, flag: Permissions) -> Permissions {
        let __raw = unsafe {
            ft_widget_add_permission(
                self.0,
                <Permissions as FfiType>::into_c(base),
                <Permissions as FfiType>::into_c(flag),
            )
        };
        <Permissions as FfiType>::from_c(__raw)
    }
    #[doc = " Consume the widget (by-value self, void return)."]
    pub fn consume(self) {
        let __handle = {
            let this = std::mem::ManuallyDrop::new(self);
            this.0
        };
        unsafe { ft_widget_consume(__handle) }
    }
    #[doc = " Get the raw fd number from a borrowed fd."]
    pub fn fd_number(&self, fd: BorrowedFd<'_>) -> i32 {
        let __raw = unsafe { ft_widget_fd_number(self.0, <BorrowedFd<'_> as FfiType>::into_c(fd)) };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Get the raw fd number, or -1 if None."]
    pub fn fd_number_optional(&self, fd: Option<BorrowedFd<'_>>) -> i32 {
        let __raw = unsafe {
            ft_widget_fd_number_optional(self.0, <Option<BorrowedFd<'_>> as FfiType>::into_c(fd))
        };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Maybe return a borrowed fd depending on `selector`:"]
    #[doc = " < 0 → error, 0 → Ok(None), > 0 → Ok(Some(stdin))."]
    pub fn maybe_fd(&self, selector: i32) -> Result<Option<BorrowedFd<'_>>, TestError> {
        let mut __out = std::mem::MaybeUninit::uninit();
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r = unsafe {
            ft_widget_maybe_fd(
                self.0,
                <i32 as FfiType>::into_c(selector),
                __out.as_mut_ptr(),
                &mut __err as *mut *mut core::ffi::c_void,
            )
        };
        if __r == 0 {
            Ok(<Option<BorrowedFd<'_>> as FfiType>::from_c(unsafe {
                __out.assume_init()
            }))
        } else {
            Err(TestError::from_ffi(__r))
        }
    }
    #[doc = " Duplicate a file descriptor (returns owned fd)."]
    pub fn dup_fd(&self, fd: BorrowedFd<'_>) -> OwnedFd {
        let __raw = unsafe { ft_widget_dup_fd(self.0, <BorrowedFd<'_> as FfiType>::into_c(fd)) };
        <OwnedFd as FfiType>::from_c(__raw)
    }
}

impl Drop for Widget {
    fn drop(&mut self) {
        unsafe { ft_widget_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_gadget_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_gadget_get(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

pub struct Gadget(*mut core::ffi::c_void);

impl Gadget {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Gadget {
    const C_HANDLE_NAME: &'static str = "Gadget";
    const TYPE_TAG: u32 = 3u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Gadget {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Gadget";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Gadget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Gadget").field(&self.0).finish()
    }
}

impl Gadget {
    #[doc = " Get the gadget value."]
    pub fn get(&self) -> i32 {
        let __raw = unsafe { ft_gadget_get(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
}

impl Drop for Gadget {
    fn drop(&mut self) {
        unsafe { ft_gadget_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_config_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_config_new() -> <Config as FfiType>::CRepr;
    pub fn ft_config_set_name(
        handle: *mut core::ffi::c_void,
        name: <&'static str as FfiType>::CRepr,
    );
    pub fn ft_config_set_size(handle: *mut core::ffi::c_void, size: <i32 as FfiType>::CRepr);
    pub fn ft_config_validated(
        handle: *mut core::ffi::c_void,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
    pub fn ft_config_get_name(handle: *mut core::ffi::c_void) -> <&'static str as FfiType>::CRepr;
    pub fn ft_config_get_size(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

pub struct Config(*mut core::ffi::c_void);

impl Config {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Config {
    const C_HANDLE_NAME: &'static str = "Config";
    const TYPE_TAG: u32 = 4u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Config {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Config";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Config").field(&self.0).finish()
    }
}

impl Config {
    #[doc = " Create a new config."]
    pub fn new() -> Config {
        let __raw = unsafe { ft_config_new() };
        <Config as FfiType>::from_c(__raw)
    }
    #[doc = " Set the name (builder pattern: consumes self, returns Self)."]
    pub fn set_name(self, name: &str) -> Self {
        let mut __handle = {
            let this = std::mem::ManuallyDrop::new(self);
            this.0
        };
        unsafe {
            ft_config_set_name(
                &mut __handle as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                <&str as FfiType>::into_c(name),
            )
        };
        Self(__handle)
    }
    #[doc = " Set the size (builder pattern)."]
    pub fn set_size(self, size: i32) -> Self {
        let mut __handle = {
            let this = std::mem::ManuallyDrop::new(self);
            this.0
        };
        unsafe {
            ft_config_set_size(
                &mut __handle as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                <i32 as FfiType>::into_c(size),
            )
        };
        Self(__handle)
    }
    #[doc = " Validate and return self, or error if name is empty."]
    pub fn validated(self) -> Result<Self, TestError> {
        let mut __handle = {
            let this = std::mem::ManuallyDrop::new(self);
            this.0
        };
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r = unsafe {
            ft_config_validated(
                &mut __handle as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                &mut __err as *mut *mut core::ffi::c_void,
            )
        };
        if __r == 0 {
            Ok(Self(__handle))
        } else {
            Err(TestError::from_ffi(__r))
        }
    }
    #[doc = " Get the config name."]
    pub fn get_name(&self) -> &str {
        let __raw = unsafe { ft_config_get_name(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    #[doc = " Get the config size."]
    pub fn get_size(&self) -> i32 {
        let __raw = unsafe { ft_config_get_size(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        unsafe { ft_config_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_gizmo_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_gizmo_name(handle: *mut core::ffi::c_void) -> <&'static str as FfiType>::CRepr;
    pub fn ft_gizmo_size(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

pub struct Gizmo(*mut core::ffi::c_void);

impl Gizmo {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Gizmo {
    const C_HANDLE_NAME: &'static str = "Gizmo";
    const TYPE_TAG: u32 = 5u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Gizmo {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Gizmo";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Gizmo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Gizmo").field(&self.0).finish()
    }
}

impl Gizmo {
    #[doc = " Get the gizmo name."]
    pub fn name(&self) -> &str {
        let __raw = unsafe { ft_gizmo_name(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    #[doc = " Get the gizmo size."]
    pub fn size(&self) -> i32 {
        let __raw = unsafe { ft_gizmo_size(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
}

impl Drop for Gizmo {
    fn drop(&mut self) {
        unsafe { ft_gizmo_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_gizmo_builder_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_gizmo_builder_new() -> <GizmoBuilder as FfiType>::CRepr;
    pub fn ft_gizmo_builder_set_name(
        handle: *mut core::ffi::c_void,
        name: <&'static str as FfiType>::CRepr,
    );
    pub fn ft_gizmo_builder_set_size(handle: *mut core::ffi::c_void, size: <i32 as FfiType>::CRepr);
    pub fn ft_gizmo_builder_build(handle: *mut core::ffi::c_void) -> <Gizmo as FfiType>::CRepr;
    pub fn ft_gizmo_builder_try_build(
        handle: *mut core::ffi::c_void,
        err_out: *mut *mut core::ffi::c_void,
    ) -> *mut core::ffi::c_void;
}

pub struct GizmoBuilder(*mut core::ffi::c_void);

impl GizmoBuilder {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for GizmoBuilder {
    const C_HANDLE_NAME: &'static str = "GizmoBuilder";
    const TYPE_TAG: u32 = 6u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for GizmoBuilder {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "GizmoBuilder";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for GizmoBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("GizmoBuilder").field(&self.0).finish()
    }
}

impl GizmoBuilder {
    #[doc = " Create a new gizmo builder."]
    pub fn new() -> GizmoBuilder {
        let __raw = unsafe { ft_gizmo_builder_new() };
        <GizmoBuilder as FfiType>::from_c(__raw)
    }
    #[doc = " Set the gizmo name."]
    pub fn set_name(&mut self, name: &str) {
        unsafe { ft_gizmo_builder_set_name(self.0, <&str as FfiType>::into_c(name)) }
    }
    #[doc = " Set the gizmo size."]
    pub fn set_size(&mut self, size: i32) {
        unsafe { ft_gizmo_builder_set_size(self.0, <i32 as FfiType>::into_c(size)) }
    }
    #[doc = " Build the gizmo (consumes builder, returns different type)."]
    pub fn build(self) -> Gizmo {
        let __handle = {
            let this = std::mem::ManuallyDrop::new(self);
            this.0
        };
        let __raw = unsafe { ft_gizmo_builder_build(__handle) };
        <Gizmo as FfiType>::from_c(__raw)
    }
    #[doc = " Try to build the gizmo; fails if name is empty."]
    pub fn try_build(self) -> Result<Gizmo, TestError> {
        let __handle = {
            let this = std::mem::ManuallyDrop::new(self);
            this.0
        };
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __raw = unsafe {
            ft_gizmo_builder_try_build(__handle, &mut __err as *mut *mut core::ffi::c_void)
        };
        if !__raw.is_null() {
            Ok(<Gizmo as FfiType>::from_c(__raw))
        } else {
            let __r = unsafe { ft_error_result(__err) };
            unsafe { ft_error_destroy(__err) };
            Err(TestError::from_ffi(__r))
        }
    }
}

impl Drop for GizmoBuilder {
    fn drop(&mut self) {
        unsafe { ft_gizmo_builder_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_view_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_view_create(
        source: <&'static Widget as FfiType>::CRepr,
    ) -> <View<'static> as FfiType>::CRepr;
    pub fn ft_view_create_labeled(
        source: <&'static Widget as FfiType>::CRepr,
        label: <&'static str as FfiType>::CRepr,
    ) -> <View<'static> as FfiType>::CRepr;
    pub fn ft_view_source_count(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
    pub fn ft_view_set_label(
        handle: *mut core::ffi::c_void,
        label: <&'static str as FfiType>::CRepr,
    );
    pub fn ft_view_label(handle: *mut core::ffi::c_void) -> <&'static str as FfiType>::CRepr;
    pub fn ft_view_copy_label(handle: *mut core::ffi::c_void, other: *mut core::ffi::c_void);
}

pub struct View<'a>(*mut core::ffi::c_void, std::marker::PhantomData<&'a ()>);

impl<'a> View<'a> {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr, std::marker::PhantomData)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl<'a> FfiHandle for View<'a> {
    const C_HANDLE_NAME: &'static str = "View";
    const TYPE_TAG: u32 = 7u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl<'a> FfiType for View<'a> {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "View";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl<'a> std::fmt::Debug for View<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("View").field(&self.0).finish()
    }
}

impl<'a> View<'a> {
    #[doc = " Create a view that borrows a widget."]
    pub fn create(source: &'a Widget) -> View<'a> {
        let __raw = unsafe { ft_view_create(FfiHandle::as_handle(source)) };
        <View<'a> as FfiType>::from_c(__raw)
    }
    #[doc = " Create a view with a custom label."]
    #[doc = ""]
    #[doc = " Takes two reference params so lifetime elision can't resolve `'_`"]
    #[doc = " in the return type — the struct lifetime must be preserved explicitly."]
    pub fn create_labeled(source: &'a Widget, label: &str) -> View<'a> {
        let __raw = unsafe {
            ft_view_create_labeled(
                FfiHandle::as_handle(source),
                <&str as FfiType>::into_c(label),
            )
        };
        <View<'a> as FfiType>::from_c(__raw)
    }
    #[doc = " Read the source widget's count through the borrow."]
    pub fn source_count(&self) -> i32 {
        let __raw = unsafe { ft_view_source_count(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Set the view label."]
    pub fn set_label(&mut self, label: &str) {
        unsafe { ft_view_set_label(self.0, <&str as FfiType>::into_c(label)) }
    }
    #[doc = " Get the view label."]
    pub fn label(&self) -> &str {
        let __raw = unsafe { ft_view_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    #[doc = " Copy label from another snapshot (tests impl Trait auto-dispatch)."]
    pub fn copy_label(&mut self, other: impl Snapshot<'a>) {
        unsafe { ft_view_copy_label(self.0, other.__into_raw_handle()) }
    }
}

impl<'a> Drop for View<'a> {
    fn drop(&mut self) {
        unsafe { ft_view_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_view_factory_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_view_factory_new() -> <ViewFactory as FfiType>::CRepr;
    pub fn ft_view_factory_create_view(
        source: <&'static Widget as FfiType>::CRepr,
        label: <&'static str as FfiType>::CRepr,
    ) -> <View<'static> as FfiType>::CRepr;
}

pub struct ViewFactory(*mut core::ffi::c_void);

impl ViewFactory {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for ViewFactory {
    const C_HANDLE_NAME: &'static str = "ViewFactory";
    const TYPE_TAG: u32 = 8u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for ViewFactory {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "ViewFactory";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for ViewFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ViewFactory").field(&self.0).finish()
    }
}

impl ViewFactory {
    pub fn new() -> ViewFactory {
        let __raw = unsafe { ft_view_factory_new() };
        <ViewFactory as FfiType>::from_c(__raw)
    }
    #[doc = " Create a view from a source widget with a label."]
    #[doc = ""]
    #[doc = " Multiple reference params + lifetime-parameterized return type forces"]
    #[doc = " the generator to introduce a method-level lifetime (can't elide)."]
    pub fn create_view<'a>(source: &'a Widget, label: &str) -> View<'a> {
        let __raw = unsafe {
            ft_view_factory_create_view(
                FfiHandle::as_handle(source),
                <&str as FfiType>::into_c(label),
            )
        };
        <View<'a> as FfiType>::from_c(__raw)
    }
}

impl Drop for ViewFactory {
    fn drop(&mut self) {
        unsafe { ft_view_factory_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_pipeline_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_pipeline_new() -> <Pipeline as FfiType>::CRepr;
    pub fn ft_pipeline_run(
        handle: *mut core::ffi::c_void,
        proc: *mut core::ffi::c_void,
        input: <i32 as FfiType>::CRepr,
    );
    pub fn ft_pipeline_result_count(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
    pub fn ft_pipeline_last_result(
        handle: *mut core::ffi::c_void,
        result: *mut <i32 as FfiType>::CRepr,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
}

pub struct Pipeline(*mut core::ffi::c_void);

impl Pipeline {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Pipeline {
    const C_HANDLE_NAME: &'static str = "Pipeline";
    const TYPE_TAG: u32 = 9u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Pipeline {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Pipeline";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Pipeline").field(&self.0).finish()
    }
}

impl Pipeline {
    #[doc = " Create a new pipeline."]
    pub fn new() -> Pipeline {
        let __raw = unsafe { ft_pipeline_new() };
        <Pipeline as FfiType>::from_c(__raw)
    }
    #[doc = " Run a processor on the given input."]
    pub fn run(&mut self, proc: impl Processor, input: i32) {
        unsafe {
            ft_pipeline_run(
                self.0,
                proc.__into_raw_handle(),
                <i32 as FfiType>::into_c(input),
            )
        }
    }
    #[doc = " Get the number of results."]
    pub fn result_count(&self) -> i32 {
        let __raw = unsafe { ft_pipeline_result_count(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Get the last result, or error if empty."]
    pub fn last_result(&self) -> Result<i32, TestError> {
        let mut __out = std::mem::MaybeUninit::uninit();
        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
        let __r = unsafe {
            ft_pipeline_last_result(
                self.0,
                __out.as_mut_ptr(),
                &mut __err as *mut *mut core::ffi::c_void,
            )
        };
        if __r == 0 {
            Ok(<i32 as FfiType>::from_c(unsafe { __out.assume_init() }))
        } else {
            Err(TestError::from_ffi(__r))
        }
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        unsafe { ft_pipeline_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_apple_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_apple_new(weight: <i32 as FfiType>::CRepr) -> <Apple as FfiType>::CRepr;
}

pub struct Apple(*mut core::ffi::c_void);

impl Apple {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Apple {
    const C_HANDLE_NAME: &'static str = "Apple";
    const TYPE_TAG: u32 = 11u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Apple {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Apple";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Apple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Apple").field(&self.0).finish()
    }
}

impl Apple {
    pub fn new(weight: i32) -> Apple {
        let __raw = unsafe { ft_apple_new(<i32 as FfiType>::into_c(weight)) };
        <Apple as FfiType>::from_c(__raw)
    }
}

impl Drop for Apple {
    fn drop(&mut self) {
        unsafe { ft_apple_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_orange_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_orange_new(juice: <i32 as FfiType>::CRepr) -> <Orange as FfiType>::CRepr;
}

pub struct Orange(*mut core::ffi::c_void);

impl Orange {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Orange {
    const C_HANDLE_NAME: &'static str = "Orange";
    const TYPE_TAG: u32 = 12u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Orange {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Orange";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Orange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Orange").field(&self.0).finish()
    }
}

impl Orange {
    pub fn new(juice: i32) -> Orange {
        let __raw = unsafe { ft_orange_new(<i32 as FfiType>::into_c(juice)) };
        <Orange as FfiType>::from_c(__raw)
    }
}

impl Drop for Orange {
    fn drop(&mut self) {
        unsafe { ft_orange_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_banana_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_banana_new(v: <i32 as FfiType>::CRepr) -> <Banana as FfiType>::CRepr;
}

pub struct Banana(*mut core::ffi::c_void);

impl Banana {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Banana {
    const C_HANDLE_NAME: &'static str = "Banana";
    const TYPE_TAG: u32 = 13u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Banana {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Banana";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Banana {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Banana").field(&self.0).finish()
    }
}

impl Banana {
    pub fn new(v: i32) -> Banana {
        let __raw = unsafe { ft_banana_new(<i32 as FfiType>::into_c(v)) };
        <Banana as FfiType>::from_c(__raw)
    }
}

impl Drop for Banana {
    fn drop(&mut self) {
        unsafe { ft_banana_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_mango_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_mango_new(v: <i32 as FfiType>::CRepr) -> <Mango as FfiType>::CRepr;
}

pub struct Mango(*mut core::ffi::c_void);

impl Mango {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Mango {
    const C_HANDLE_NAME: &'static str = "Mango";
    const TYPE_TAG: u32 = 14u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Mango {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Mango";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Mango {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Mango").field(&self.0).finish()
    }
}

impl Mango {
    pub fn new(v: i32) -> Mango {
        let __raw = unsafe { ft_mango_new(<i32 as FfiType>::into_c(v)) };
        <Mango as FfiType>::from_c(__raw)
    }
}

impl Drop for Mango {
    fn drop(&mut self) {
        unsafe { ft_mango_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_peach_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_peach_new(v: <i32 as FfiType>::CRepr) -> <Peach as FfiType>::CRepr;
}

pub struct Peach(*mut core::ffi::c_void);

impl Peach {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Peach {
    const C_HANDLE_NAME: &'static str = "Peach";
    const TYPE_TAG: u32 = 15u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Peach {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Peach";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Peach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Peach").field(&self.0).finish()
    }
}

impl Peach {
    pub fn new(v: i32) -> Peach {
        let __raw = unsafe { ft_peach_new(<i32 as FfiType>::into_c(v)) };
        <Peach as FfiType>::from_c(__raw)
    }
}

impl Drop for Peach {
    fn drop(&mut self) {
        unsafe { ft_peach_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_plum_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_plum_new(v: <i32 as FfiType>::CRepr) -> <Plum as FfiType>::CRepr;
}

pub struct Plum(*mut core::ffi::c_void);

impl Plum {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Plum {
    const C_HANDLE_NAME: &'static str = "Plum";
    const TYPE_TAG: u32 = 16u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Plum {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Plum";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Plum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Plum").field(&self.0).finish()
    }
}

impl Plum {
    pub fn new(v: i32) -> Plum {
        let __raw = unsafe { ft_plum_new(<i32 as FfiType>::into_c(v)) };
        <Plum as FfiType>::from_c(__raw)
    }
}

impl Drop for Plum {
    fn drop(&mut self) {
        unsafe { ft_plum_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_grape_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_grape_new(v: <i32 as FfiType>::CRepr) -> <Grape as FfiType>::CRepr;
}

pub struct Grape(*mut core::ffi::c_void);

impl Grape {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Grape {
    const C_HANDLE_NAME: &'static str = "Grape";
    const TYPE_TAG: u32 = 17u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Grape {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Grape";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Grape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Grape").field(&self.0).finish()
    }
}

impl Grape {
    pub fn new(v: i32) -> Grape {
        let __raw = unsafe { ft_grape_new(<i32 as FfiType>::into_c(v)) };
        <Grape as FfiType>::from_c(__raw)
    }
}

impl Drop for Grape {
    fn drop(&mut self) {
        unsafe { ft_grape_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_lemon_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_lemon_new(v: <i32 as FfiType>::CRepr) -> <Lemon as FfiType>::CRepr;
}

pub struct Lemon(*mut core::ffi::c_void);

impl Lemon {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Lemon {
    const C_HANDLE_NAME: &'static str = "Lemon";
    const TYPE_TAG: u32 = 18u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Lemon {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Lemon";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Lemon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Lemon").field(&self.0).finish()
    }
}

impl Lemon {
    pub fn new(v: i32) -> Lemon {
        let __raw = unsafe { ft_lemon_new(<i32 as FfiType>::into_c(v)) };
        <Lemon as FfiType>::from_c(__raw)
    }
}

impl Drop for Lemon {
    fn drop(&mut self) {
        unsafe { ft_lemon_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_mixer_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_mixer_new() -> <Mixer as FfiType>::CRepr;
    pub fn ft_mixer_add(handle: *mut core::ffi::c_void, fruit: *mut core::ffi::c_void);
    pub fn ft_mixer_fruit_label_len(
        handle: *mut core::ffi::c_void,
        fruit: *mut core::ffi::c_void,
    ) -> <i32 as FfiType>::CRepr;
    pub fn ft_mixer_blend_concrete(
        handle: *mut core::ffi::c_void,
        a: *mut core::ffi::c_void,
        b: *mut core::ffi::c_void,
    ) -> <i32 as FfiType>::CRepr;
    pub fn ft_mixer_blend_hybrid(
        handle: *mut core::ffi::c_void,
        a: *mut core::ffi::c_void,
        b: *mut core::ffi::c_void,
    ) -> <i32 as FfiType>::CRepr;
    pub fn ft_mixer_blend_dynamic(
        handle: *mut core::ffi::c_void,
        a: *mut core::ffi::c_void,
        b: *mut core::ffi::c_void,
    ) -> <i32 as FfiType>::CRepr;
    pub fn ft_mixer_total(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

pub struct Mixer(*mut core::ffi::c_void);

impl Mixer {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Mixer {
    const C_HANDLE_NAME: &'static str = "Mixer";
    const TYPE_TAG: u32 = 21u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Mixer {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Mixer";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Mixer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Mixer").field(&self.0).finish()
    }
}

impl Mixer {
    pub fn new() -> Mixer {
        let __raw = unsafe { ft_mixer_new() };
        <Mixer as FfiType>::from_c(__raw)
    }
    pub fn add(self, fruit: impl Fruit) -> Self {
        let mut __handle = {
            let this = std::mem::ManuallyDrop::new(self);
            this.0
        };
        unsafe {
            ft_mixer_add(
                &mut __handle as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                fruit.__into_raw_handle(),
            )
        };
        Self(__handle)
    }
    #[doc = " Returns the length of a fruit's label. Used to test that vtable"]
    #[doc = " default method detection works for custom client types crossing FFI."]
    pub fn fruit_label_len(&self, fruit: impl Fruit) -> i32 {
        let __raw = unsafe { ft_mixer_fruit_label_len(self.0, fruit.__into_raw_handle()) };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Both concrete (9^2=81 > 64, override with annotation)."]
    pub fn blend_concrete(&mut self, a: impl Fruit, b: impl Fruit) -> i32 {
        let __raw = unsafe {
            ft_mixer_blend_concrete(self.0, a.__into_raw_handle(), b.__into_raw_handle())
        };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " First concrete, second vtable (hybrid: 9+9=18 branches)."]
    pub fn blend_hybrid(&mut self, a: impl Fruit, b: impl Fruit) -> i32 {
        let __raw =
            unsafe { ft_mixer_blend_hybrid(self.0, a.__into_raw_handle(), b.__into_raw_handle()) };
        <i32 as FfiType>::from_c(__raw)
    }
    #[doc = " Both vtable (9+9=18 branches)."]
    pub fn blend_dynamic(&mut self, a: impl Fruit, b: impl Fruit) -> i32 {
        let __raw =
            unsafe { ft_mixer_blend_dynamic(self.0, a.__into_raw_handle(), b.__into_raw_handle()) };
        <i32 as FfiType>::from_c(__raw)
    }
    pub fn total(&self) -> i32 {
        let __raw = unsafe { ft_mixer_total(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
}

impl Drop for Mixer {
    fn drop(&mut self) {
        unsafe { ft_mixer_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn ft_sprocket_destroy(handle: *mut core::ffi::c_void);
    pub fn ft_sprocket_new(name: <&'static str as FfiType>::CRepr) -> <Sprocket as FfiType>::CRepr;
}

pub struct Sprocket(*mut core::ffi::c_void);

impl Sprocket {
    #[doc(hidden)]
    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {
        Self(ptr)
    }
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl FfiHandle for Sprocket {
    const C_HANDLE_NAME: &'static str = "Sprocket";
    const TYPE_TAG: u32 = 22u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl FfiType for Sprocket {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "Sprocket";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for Sprocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Sprocket").field(&self.0).finish()
    }
}

impl Sprocket {
    pub fn new(name: &str) -> Sprocket {
        let __raw = unsafe { ft_sprocket_new(<&str as FfiType>::into_c(name)) };
        <Sprocket as FfiType>::from_c(__raw)
    }
}

impl Drop for Sprocket {
    fn drop(&mut self) {
        unsafe { ft_sprocket_destroy(self.0) }
    }
}

pub trait Processor {
    fn process(&self, input: i32) -> i32;
    fn name(&self) -> &str;
    fn on_notify(&self, code: i32);
    #[doc(hidden)]
    fn __ffier_vtable() -> &'static ProcessorVtable
    where
        Self: Sized,
    {
        &ProcessorVtable {
            drop: Some({
                unsafe extern "C" fn __drop_trampoline<__T>(__ud: *mut core::ffi::c_void) {
                    unsafe { drop(Box::from_raw(__ud as *mut __T)) };
                }
                __drop_trampoline::<Self>
            }),
            process: Some({
                unsafe extern "C" fn __trampoline<__T: Processor>(
                    __ud: *mut core::ffi::c_void,
                    input: <i32 as FfiType>::CRepr,
                ) -> <i32 as FfiType>::CRepr {
                    let __val = unsafe { &*(__ud as *const __T) };
                    let __result = __val.process(<i32 as FfiType>::from_c(input));
                    <i32 as FfiType>::into_c(__result)
                }
                __trampoline::<Self>
            }),
            name: Some({
                unsafe extern "C" fn __trampoline<__T: Processor>(
                    __ud: *mut core::ffi::c_void,
                ) -> <&'static str as FfiType>::CRepr {
                    let __val = unsafe { &*(__ud as *const __T) };
                    let __result = __val.name();
                    <&str as FfiType>::into_c(__result)
                }
                __trampoline::<Self>
            }),
            on_notify: Some({
                unsafe extern "C" fn __trampoline<__T: Processor>(
                    __ud: *mut core::ffi::c_void,
                    code: <i32 as FfiType>::CRepr,
                ) {
                    let __val = unsafe { &*(__ud as *const __T) };
                    let __result = __val.on_notify(<i32 as FfiType>::from_c(code));
                    __result
                }
                __trampoline::<Self>
            }),
        }
    }
    #[doc(hidden)]
    fn __into_raw_handle(self) -> *mut core::ffi::c_void
    where
        Self: Sized,
    {
        let __vtable: &'static ProcessorVtable = Self::__ffier_vtable();
        let __user_data = Box::into_raw(Box::new(self));
        let vtable_size: u16 = core::mem::size_of::<ProcessorVtable>()
            .try_into()
            .expect("vtable_size exceeds u16::MAX");
        ffier::ffier_handle_new_with_metadata(
            10u32,
            0,
            ffier::VtableHandle {
                vtable_ptr: __vtable as *const ProcessorVtable as *const core::ffi::c_void,
                user_data: __user_data as *const core::ffi::c_void,
                vtable_size,
            },
        )
    }
}

#[repr(C)]
pub struct ProcessorVtable {
    pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
    pub process: Option<
        unsafe extern "C" fn(
            *mut core::ffi::c_void,
            <i32 as FfiType>::CRepr,
        ) -> <i32 as FfiType>::CRepr,
    >,
    pub name:
        Option<unsafe extern "C" fn(*mut core::ffi::c_void) -> <&'static str as FfiType>::CRepr>,
    pub on_notify: Option<unsafe extern "C" fn(*mut core::ffi::c_void, <i32 as FfiType>::CRepr)>,
}

pub struct VtableProcessor(*mut core::ffi::c_void);

impl VtableProcessor {
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl Drop for VtableProcessor {
    fn drop(&mut self) {}
}

pub trait Fruit {
    fn value(&self) -> i32;
    fn label(&self) -> &str
    where
        Self: Sized,
    {
        let __vtable: &'static FruitVtable = Self::__ffier_vtable();
        let __metadata: u32 = 2 | (1u32 << 2);
        let mut __temp = ffier::FfierHandle {
            type_tag: 20u32,
            metadata: __metadata,
            value: ffier::VtableHandle {
                vtable_ptr: __vtable as *const FruitVtable as *const core::ffi::c_void,
                user_data: self as *const Self as *const core::ffi::c_void,
                vtable_size: core::mem::size_of::<FruitVtable>() as u16,
            },
        };
        let __raw = unsafe {
            ft_fruit_label(
                &mut __temp as *mut ffier::FfierHandle<ffier::VtableHandle>
                    as *mut core::ffi::c_void,
            )
        };
        <&str as FfiType>::from_c(__raw)
    }
    #[doc(hidden)]
    fn __ffier_vtable() -> &'static FruitVtable
    where
        Self: Sized,
    {
        &FruitVtable {
            drop: Some({
                unsafe extern "C" fn __drop_trampoline<__T>(__ud: *mut core::ffi::c_void) {
                    unsafe { drop(Box::from_raw(__ud as *mut __T)) };
                }
                __drop_trampoline::<Self>
            }),
            value: Some({
                unsafe extern "C" fn __trampoline<__T: Fruit>(
                    __ud: *mut core::ffi::c_void,
                ) -> <i32 as FfiType>::CRepr {
                    let __val = unsafe { &*(__ud as *const __T) };
                    let __result = __val.value();
                    <i32 as FfiType>::into_c(__result)
                }
                __trampoline::<Self>
            }),
            label: Some({
                unsafe extern "C" fn __trampoline<__T: Fruit>(
                    __ud: *mut core::ffi::c_void,
                ) -> <&'static str as FfiType>::CRepr {
                    let __val = unsafe { &*(__ud as *const __T) };
                    let __result = __val.label();
                    <&str as FfiType>::into_c(__result)
                }
                __trampoline::<Self>
            }),
        }
    }
    #[doc(hidden)]
    fn __into_raw_handle(self) -> *mut core::ffi::c_void
    where
        Self: Sized,
    {
        let __vtable: &'static FruitVtable = Self::__ffier_vtable();
        let __user_data = Box::into_raw(Box::new(self));
        let vtable_size: u16 = core::mem::size_of::<FruitVtable>()
            .try_into()
            .expect("vtable_size exceeds u16::MAX");
        ffier::ffier_handle_new_with_metadata(
            20u32,
            0,
            ffier::VtableHandle {
                vtable_ptr: __vtable as *const FruitVtable as *const core::ffi::c_void,
                user_data: __user_data as *const core::ffi::c_void,
                vtable_size,
            },
        )
    }
}

#[repr(C)]
pub struct FruitVtable {
    pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
    pub value: Option<unsafe extern "C" fn(*mut core::ffi::c_void) -> <i32 as FfiType>::CRepr>,
    pub label:
        Option<unsafe extern "C" fn(*mut core::ffi::c_void) -> <&'static str as FfiType>::CRepr>,
}

unsafe extern "C" {
    pub fn ft_fruit_label(handle: *mut core::ffi::c_void) -> <&'static str as FfiType>::CRepr;
}

pub struct VtableFruit(*mut core::ffi::c_void);

impl VtableFruit {
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl Drop for VtableFruit {
    fn drop(&mut self) {}
}

pub trait Weighable {
    fn weight_grams(&self) -> i32;
    #[doc(hidden)]
    fn __ffier_vtable() -> &'static WeighableVtable
    where
        Self: Sized,
    {
        &WeighableVtable {
            drop: Some({
                unsafe extern "C" fn __drop_trampoline<__T>(__ud: *mut core::ffi::c_void) {
                    unsafe { drop(Box::from_raw(__ud as *mut __T)) };
                }
                __drop_trampoline::<Self>
            }),
            weight_grams: Some({
                unsafe extern "C" fn __trampoline<__T: Weighable>(
                    __ud: *mut core::ffi::c_void,
                ) -> <i32 as FfiType>::CRepr {
                    let __val = unsafe { &*(__ud as *const __T) };
                    let __result = __val.weight_grams();
                    <i32 as FfiType>::into_c(__result)
                }
                __trampoline::<Self>
            }),
        }
    }
    #[doc(hidden)]
    fn __into_raw_handle(self) -> *mut core::ffi::c_void
    where
        Self: Sized,
    {
        let __vtable: &'static WeighableVtable = Self::__ffier_vtable();
        let __user_data = Box::into_raw(Box::new(self));
        let vtable_size: u16 = core::mem::size_of::<WeighableVtable>()
            .try_into()
            .expect("vtable_size exceeds u16::MAX");
        ffier::ffier_handle_new_with_metadata(
            23u32,
            0,
            ffier::VtableHandle {
                vtable_ptr: __vtable as *const WeighableVtable as *const core::ffi::c_void,
                user_data: __user_data as *const core::ffi::c_void,
                vtable_size,
            },
        )
    }
}

#[repr(C)]
pub struct WeighableVtable {
    pub drop: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>,
    pub weight_grams:
        Option<unsafe extern "C" fn(*mut core::ffi::c_void) -> <i32 as FfiType>::CRepr>,
}

pub struct VtableWeighable(*mut core::ffi::c_void);

impl VtableWeighable {
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl Drop for VtableWeighable {
    fn drop(&mut self) {}
}

pub trait PushStr {
    fn push(&mut self, s: &str) -> bool;
    #[doc(hidden)]
    fn __ffier_vtable() -> &'static PushStrVtable
    where
        Self: Sized,
    {
        &PushStrVtable {
            drop: Some({
                unsafe extern "C" fn __drop_trampoline<__T>(__ud: *mut core::ffi::c_void) {
                    unsafe { drop(Box::from_raw(__ud as *mut __T)) };
                }
                __drop_trampoline::<Self>
            }),
            push: Some({
                unsafe extern "C" fn __trampoline<__T: PushStr>(
                    __ud: *mut core::ffi::c_void,
                    s: <&'static str as FfiType>::CRepr,
                ) -> <bool as FfiType>::CRepr {
                    let __val = unsafe { &mut *(__ud as *mut __T) };
                    let __result = __val.push(<&str as FfiType>::from_c(s));
                    <bool as FfiType>::into_c(__result)
                }
                __trampoline::<Self>
            }),
        }
    }
    #[doc(hidden)]
    fn __into_raw_handle(self) -> *mut core::ffi::c_void
    where
        Self: Sized,
    {
        let __vtable: &'static PushStrVtable = Self::__ffier_vtable();
        let __user_data = Box::into_raw(Box::new(self));
        let vtable_size: u16 = core::mem::size_of::<PushStrVtable>()
            .try_into()
            .expect("vtable_size exceeds u16::MAX");
        ffier::ffier_handle_new_with_metadata(
            24u32,
            0,
            ffier::VtableHandle {
                vtable_ptr: __vtable as *const PushStrVtable as *const core::ffi::c_void,
                user_data: __user_data as *const core::ffi::c_void,
                vtable_size,
            },
        )
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

pub struct VtablePushStr(*mut core::ffi::c_void);

impl VtablePushStr {
    #[doc(hidden)]
    pub fn __into_raw(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

impl Drop for VtablePushStr {
    fn drop(&mut self) {}
}

unsafe extern "C" {
    pub fn ft_error_code(handle: *mut core::ffi::c_void) -> <u32 as FfiType>::CRepr;
    pub fn ft_error_message(handle: *mut core::ffi::c_void, writer: *mut core::ffi::c_void);
    pub fn ft_error_result(handle: *mut core::ffi::c_void) -> <u64 as FfiType>::CRepr;
    pub fn ft_error_destroy(handle: *mut core::ffi::c_void);
}

unsafe extern "C" {
    pub fn ft_apple_value(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Fruit for Apple {
    fn value(&self) -> i32 {
        let __raw = unsafe { ft_apple_value(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn label(&self) -> &str {
        let __raw = unsafe { ft_fruit_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_orange_value(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Fruit for Orange {
    fn value(&self) -> i32 {
        let __raw = unsafe { ft_orange_value(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn label(&self) -> &str {
        let __raw = unsafe { ft_fruit_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_banana_value(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Fruit for Banana {
    fn value(&self) -> i32 {
        let __raw = unsafe { ft_banana_value(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn label(&self) -> &str {
        let __raw = unsafe { ft_fruit_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_mango_value(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Fruit for Mango {
    fn value(&self) -> i32 {
        let __raw = unsafe { ft_mango_value(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn label(&self) -> &str {
        let __raw = unsafe { ft_fruit_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_peach_value(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Fruit for Peach {
    fn value(&self) -> i32 {
        let __raw = unsafe { ft_peach_value(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn label(&self) -> &str {
        let __raw = unsafe { ft_fruit_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_plum_value(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Fruit for Plum {
    fn value(&self) -> i32 {
        let __raw = unsafe { ft_plum_value(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn label(&self) -> &str {
        let __raw = unsafe { ft_fruit_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_grape_value(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Fruit for Grape {
    fn value(&self) -> i32 {
        let __raw = unsafe { ft_grape_value(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn label(&self) -> &str {
        let __raw = unsafe { ft_fruit_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_lemon_value(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Fruit for Lemon {
    fn value(&self) -> i32 {
        let __raw = unsafe { ft_lemon_value(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn label(&self) -> &str {
        let __raw = unsafe { ft_fruit_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

pub trait Attachment {
    fn label(&self) -> &str;
    #[doc(hidden)]
    fn __into_raw_handle(self) -> *mut core::ffi::c_void
    where
        Self: Sized;
}

unsafe extern "C" {
    pub fn ft_sprocket_label(handle: *mut core::ffi::c_void) -> <&'static str as FfiType>::CRepr;
}

impl Attachment for Sprocket {
    fn label(&self) -> &str {
        let __raw = unsafe { ft_sprocket_label(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

pub trait Snapshot<'a> {
    fn snap_description(&self) -> &str;
    fn snap_source_count(&self) -> i32;
    #[doc(hidden)]
    fn __into_raw_handle(self) -> *mut core::ffi::c_void
    where
        Self: Sized;
}

unsafe extern "C" {
    pub fn ft_view_snap_description(
        handle: *mut core::ffi::c_void,
    ) -> <&'static str as FfiType>::CRepr;
    pub fn ft_view_snap_source_count(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl<'a> Snapshot<'a> for View<'a> {
    fn snap_description(&self) -> &str {
        let __raw = unsafe { ft_view_snap_description(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn snap_source_count(&self) -> i32 {
        let __raw = unsafe { ft_view_snap_source_count(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_widget_snap_description(
        handle: *mut core::ffi::c_void,
    ) -> <&'static str as FfiType>::CRepr;
    pub fn ft_widget_snap_source_count(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Snapshot<'static> for Widget {
    fn snap_description(&self) -> &str {
        let __raw = unsafe { ft_widget_snap_description(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn snap_source_count(&self) -> i32 {
        let __raw = unsafe { ft_widget_snap_source_count(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_gadget_snap_description(
        handle: *mut core::ffi::c_void,
    ) -> <&'static str as FfiType>::CRepr;
    pub fn ft_gadget_snap_source_count(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl<'a> Snapshot<'a> for Gadget {
    fn snap_description(&self) -> &str {
        let __raw = unsafe { ft_gadget_snap_description(self.0) };
        <&str as FfiType>::from_c(__raw)
    }
    fn snap_source_count(&self) -> i32 {
        let __raw = unsafe { ft_gadget_snap_source_count(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_apple_weight_grams(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

impl Weighable for Apple {
    fn weight_grams(&self) -> i32 {
        let __raw = unsafe { ft_apple_weight_grams(self.0) };
        <i32 as FfiType>::from_c(__raw)
    }
    fn __into_raw_handle(self) -> *mut core::ffi::c_void {
        let this = std::mem::ManuallyDrop::new(self);
        this.0
    }
}

unsafe extern "C" {
    pub fn ft_log_level_name(
        level: <LogLevel as FfiType>::CRepr,
    ) -> <&'static str as FfiType>::CRepr;
}

#[doc = " Describe a log level as a string."]
pub fn log_level_name(level: LogLevel) -> &'static str {
    let __raw = unsafe { ft_log_level_name(<LogLevel as FfiType>::into_c(level)) };
    <&'static str as FfiType>::from_c(__raw)
}

unsafe extern "C" {
    pub fn ft_log_level_is_enabled(level: <LogLevel as FfiType>::CRepr)
        -> <bool as FfiType>::CRepr;
}

#[doc = " Check if a log level is enabled (everything above Off)."]
pub fn log_level_is_enabled(level: LogLevel) -> bool {
    let __raw = unsafe { ft_log_level_is_enabled(<LogLevel as FfiType>::into_c(level)) };
    <bool as FfiType>::from_c(__raw)
}

unsafe extern "C" {
    pub fn ft_clone_fd(
        fd: <BorrowedFd<'static> as FfiType>::CRepr,
        result: *mut <OwnedFd as FfiType>::CRepr,
        err_out: *mut *mut core::ffi::c_void,
    ) -> ffier::FfierResult;
}

#[doc = " Duplicate a file descriptor."]
pub fn clone_fd(fd: BorrowedFd<'_>) -> Result<OwnedFd, TestError> {
    let mut __out = std::mem::MaybeUninit::uninit();
    let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();
    let __r = unsafe {
        ft_clone_fd(
            <BorrowedFd<'_> as FfiType>::into_c(fd),
            __out.as_mut_ptr(),
            &mut __err as *mut *mut core::ffi::c_void,
        )
    };
    if __r == 0 {
        Ok(<OwnedFd as FfiType>::from_c(unsafe { __out.assume_init() }))
    } else {
        Err(TestError::from_ffi(__r))
    }
}
