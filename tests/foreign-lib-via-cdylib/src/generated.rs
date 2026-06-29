/// Marker trait for types exported as opaque C handles.
pub trait FfiHandle {
    const C_HANDLE_NAME: &'static str;
    const TYPE_TAG: u32;
    /// # Safety
    /// The caller must ensure the handle is not used after the object is dropped.
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void;
    fn __from_raw(handle: *mut core::ffi::c_void) -> Self;
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
    isize => "ssize_t", usize => "size_t", bool => "bool",
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
                core::mem::ManuallyDrop::new(T::__from_raw(unsafe {
                    ffier::ffier_object_array_get(raw, i)
                }))
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

unsafe extern "C" {
    fn fl_error_payload(
        handle: *const core::ffi::c_void,
        out_buf: *mut core::ffi::c_void,
        buf_size: usize,
    );
}

pub struct ForeignErrorErrorHandle(*mut core::ffi::c_void);
impl ForeignErrorErrorHandle {
    fn handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
}
impl Drop for ForeignErrorErrorHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { fl_error_destroy(self.0) }
        }
    }
}
impl std::fmt::Debug for ForeignErrorErrorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ErrorHandle({:?})", self.0)
    }
}

#[derive(Debug)]
pub enum ForeignError {
    Invalid(ForeignErrorErrorHandle),
}

impl ForeignError {
    pub fn from_ffi(r: ffier::FfierResult, err_handle: *mut core::ffi::c_void) -> Self {
        let code = ffier::ffier_result_code(r);
        let handle = ForeignErrorErrorHandle(err_handle);
        match code {
            1u32 => Self::Invalid(handle),
            other => panic!("unknown {} error code {}", "ForeignError", other),
        }
    }
    fn handle_ptr(&self) -> *mut core::ffi::c_void {
        match self {
            Self::Invalid(h) => h.handle(),
        }
    }
}

impl std::fmt::Display for ForeignError {
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
        let __vtable: &'static PushStrVtable = FmtWriter::__ffier_vtable();
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
        unsafe { fl_error_message(self.handle_ptr(), __writer_handle) };
        Ok(())
    }
}

impl std::error::Error for ForeignError {}

unsafe extern "C" {
    pub fn fl_foreign_item_destroy(handle: *mut core::ffi::c_void);
    pub fn fl_foreign_item_new(
        label: <&'static str as FfiType>::CRepr,
        score: <i32 as FfiType>::CRepr,
    ) -> <ForeignItem as FfiType>::CRepr;
    pub fn fl_foreign_item_label(
        handle: *mut core::ffi::c_void,
    ) -> <&'static str as FfiType>::CRepr;
    pub fn fl_foreign_item_score(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
    pub fn fl_foreign_item_set_score(
        handle: *mut core::ffi::c_void,
        score: <i32 as FfiType>::CRepr,
    );
}

pub struct ForeignItem(*mut core::ffi::c_void);

impl ForeignItem {
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

impl FfiHandle for ForeignItem {
    const C_HANDLE_NAME: &'static str = "FlForeignItem";
    const TYPE_TAG: u32 = 33554434u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
    fn __from_raw(handle: *mut core::ffi::c_void) -> Self {
        Self(handle)
    }
}

impl FfiType for ForeignItem {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "ForeignItem";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for ForeignItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ForeignItem").field(&self.0).finish()
    }
}

impl ForeignItem {
    pub fn new(label: &str, score: i32) -> ForeignItem {
        let __raw = unsafe {
            fl_foreign_item_new(
                <&str as FfiType>::into_c(label),
                <i32 as FfiType>::into_c(score),
            )
        };
        unsafe { <ForeignItem as FfiType>::from_c(__raw) }
    }
    pub fn label(&self) -> &str {
        let __raw = unsafe { fl_foreign_item_label(self.0) };
        unsafe { <&str as FfiType>::from_c(__raw) }
    }
    pub fn score(&self) -> i32 {
        let __raw = unsafe { fl_foreign_item_score(self.0) };
        unsafe { <i32 as FfiType>::from_c(__raw) }
    }
    pub fn set_score(&mut self, score: i32) {
        unsafe { fl_foreign_item_set_score(self.0, <i32 as FfiType>::into_c(score)) }
    }
}

impl Drop for ForeignItem {
    fn drop(&mut self) {
        unsafe { fl_foreign_item_destroy(self.0) }
    }
}

unsafe extern "C" {
    pub fn fl_foreign_config_destroy(handle: *mut core::ffi::c_void);
    pub fn fl_foreign_config_new(
        name: <&'static str as FfiType>::CRepr,
        value: <i32 as FfiType>::CRepr,
    ) -> <ForeignConfig as FfiType>::CRepr;
    pub fn fl_foreign_config_name(
        handle: *mut core::ffi::c_void,
    ) -> <&'static str as FfiType>::CRepr;
    pub fn fl_foreign_config_value(handle: *mut core::ffi::c_void) -> <i32 as FfiType>::CRepr;
}

pub struct ForeignConfig(*mut core::ffi::c_void);

impl ForeignConfig {
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

impl FfiHandle for ForeignConfig {
    const C_HANDLE_NAME: &'static str = "FlForeignConfig";
    const TYPE_TAG: u32 = 33554435u32;
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {
        self.0
    }
    fn __from_raw(handle: *mut core::ffi::c_void) -> Self {
        Self(handle)
    }
}

impl FfiType for ForeignConfig {
    type CRepr = *mut core::ffi::c_void;
    const C_TYPE_NAME: &'static str = "ForeignConfig";
    fn into_c(self) -> *mut core::ffi::c_void {
        self.__into_raw()
    }
    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {
        Self::__from_raw(repr)
    }
}

impl std::fmt::Debug for ForeignConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ForeignConfig").field(&self.0).finish()
    }
}

impl ForeignConfig {
    pub fn new(name: &str, value: i32) -> ForeignConfig {
        let __raw = unsafe {
            fl_foreign_config_new(
                <&str as FfiType>::into_c(name),
                <i32 as FfiType>::into_c(value),
            )
        };
        unsafe { <ForeignConfig as FfiType>::from_c(__raw) }
    }
    pub fn name(&self) -> &str {
        let __raw = unsafe { fl_foreign_config_name(self.0) };
        unsafe { <&str as FfiType>::from_c(__raw) }
    }
    pub fn value(&self) -> i32 {
        let __raw = unsafe { fl_foreign_config_value(self.0) };
        unsafe { <i32 as FfiType>::from_c(__raw) }
    }
}

impl Drop for ForeignConfig {
    fn drop(&mut self) {
        unsafe { fl_foreign_config_destroy(self.0) }
    }
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
                    let __result = __val.push(unsafe { <&str as FfiType>::from_c(s) });
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
            33554436u32,
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
    pub fn fl_error_code(handle: *mut core::ffi::c_void) -> <u32 as FfiType>::CRepr;
    pub fn fl_error_message(handle: *mut core::ffi::c_void, writer: *mut core::ffi::c_void);
    pub fn fl_error_result(handle: *mut core::ffi::c_void) -> <u64 as FfiType>::CRepr;
    pub fn fl_error_destroy(handle: *mut core::ffi::c_void);
}
