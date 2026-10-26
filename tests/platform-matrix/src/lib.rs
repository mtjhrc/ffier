#![recursion_limit = "256"]

#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle, RawHandle};

#[derive(Default)]
pub struct HandleApi;

/// Minimal error type for the fallible resource methods.
#[derive(Debug, PartialEq, Eq, thiserror::Error, ffier::FfiError)]
#[non_exhaustive]
pub enum ResourceError {
    #[error("invalid input")]
    #[ffier(code = 1, message = "invalid input")]
    InvalidInput(),
}

#[cfg(feature = "owner-api")]
#[derive(Default)]
pub struct OptionalApi;

#[ffier::export(cfg = "feature = \"owner-api\"")]
impl OptionalApi {
    #[cfg(unix)]
    pub fn new() -> Self {
        Self
    }

    #[cfg(windows)]
    pub fn new() -> Self {
        Self
    }

    #[cfg(all(unix, feature = "owner-api"))]
    pub fn resource(&self, fd: BorrowedFd<'_>) -> RawFd {
        fd.as_raw_fd()
    }

    #[cfg(windows)]
    pub fn resource(&self, handle: BorrowedHandle<'_>) -> RawHandle {
        handle.as_raw_handle()
    }
}

#[cfg(windows)]
#[derive(Default)]
pub struct WindowsValue;
#[ffier::export(cfg = "windows")]
impl WindowsValue {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(unix)]
#[derive(Default)]
pub struct UnixValue;
#[ffier::export(cfg = "unix")]
impl UnixValue {
    pub fn new() -> Self {
        Self
    }
}

#[ffier::export(cfg = "windows")]
pub fn windows_value(_value: WindowsValue) -> usize {
    11
}

#[ffier::export(cfg = "unix")]
pub fn unix_value(_value: UnixValue) -> usize {
    1
}

#[ffier::export(cfg = "unix,")]
pub fn resource(handle: std::os::fd::BorrowedFd<'_>) -> std::os::fd::RawFd {
    use std::os::fd::AsRawFd;
    handle.as_raw_fd()
}

#[ffier::export(cfg = "windows,")]
pub fn resource(
    handle: std::os::windows::io::BorrowedHandle<'_>,
) -> std::os::windows::io::RawHandle {
    use std::os::windows::io::AsRawHandle;
    handle.as_raw_handle()
}

#[cfg(unix)]
#[cfg(true)]
#[ffier::export]
pub fn direct_resource(handle: std::os::fd::BorrowedFd<'_>) -> std::os::fd::RawFd {
    use std::os::fd::AsRawFd;
    handle.as_raw_fd()
}

#[cfg(windows)]
#[cfg(true)]
#[ffier::export]
pub fn direct_resource(
    handle: std::os::windows::io::BorrowedHandle<'_>,
) -> std::os::windows::io::RawHandle {
    use std::os::windows::io::AsRawHandle;
    handle.as_raw_handle()
}

#[cfg(feature = "wide")]
#[ffier::export]
pub fn feature_width(value: i64) -> i64 {
    value + 8
}

#[cfg(not(feature = "wide"))]
#[ffier::export]
pub fn feature_width(value: i32) -> i32 {
    value + 4
}

#[cfg_attr(feature = "wide", ffier::export)]
#[cfg_attr(not(feature = "wide"), ffier::export)]
pub fn conditional_export() -> i32 {
    77
}

pub mod unrelated {
    #[ffier::export(cfg = "windows")]
    pub fn generated_value() -> i32 {
        1
    }
}

macro_rules! define_export {
    () => {
        #[ffier::export]
        pub fn generated_value() -> i32 {
            1
        }
    };
}
define_export!();

#[ffier::export]
impl HandleApi {
    pub fn new() -> Self {
        Self
    }

    #[cfg(all(unix, true))]
    pub fn something_with_console(&self, fd: BorrowedFd<'_>) -> RawFd {
        fd.as_raw_fd()
    }

    #[cfg(windows)]
    pub fn something_with_windows(&self, handle: BorrowedHandle<'_>) -> RawHandle {
        handle.as_raw_handle()
    }

    #[cfg(unix)]
    pub fn resource(&self, fd: BorrowedFd<'_>) -> RawFd {
        fd.as_raw_fd()
    }

    #[cfg(windows)]
    pub fn resource(&self, handle: BorrowedHandle<'_>) -> RawHandle {
        handle.as_raw_handle()
    }

    #[cfg(windows)]
    pub fn value(_value: WindowsValue) -> usize {
        11
    }

    #[cfg(unix)]
    pub fn value(_value: UnixValue) -> usize {
        1
    }
    #[cfg(windows)]
    pub fn echo<'a>(handle: Option<BorrowedHandle<'a>>) -> Option<BorrowedHandle<'a>> {
        handle
    }

    #[cfg(unix)]
    pub fn echo<'a>(handle: Option<BorrowedFd<'a>>) -> Option<BorrowedFd<'a>> {
        handle
    }

    #[cfg(target_os = "windows")]
    pub fn raw(handle: BorrowedHandle<'_>) -> RawHandle {
        handle.as_raw_handle()
    }

    #[cfg(target_os = "windows")]
    pub fn optional_raw(handle: Option<BorrowedHandle<'_>>) -> RawHandle {
        handle.map_or(core::ptr::null_mut(), |handle| handle.as_raw_handle())
    }

    #[cfg(target_os = "windows")]
    pub fn take(handle: OwnedHandle) -> OwnedHandle {
        handle
    }

    #[cfg(target_os = "windows")]
    pub fn optional_take(handle: Option<OwnedHandle>) -> Option<OwnedHandle> {
        handle
    }

    #[cfg(unix)]
    pub fn raw(handle: BorrowedFd<'_>) -> RawFd {
        handle.as_raw_fd()
    }

    #[cfg(unix)]
    pub fn optional_raw(handle: Option<BorrowedFd<'_>>) -> RawFd {
        handle.map_or(-1, |handle| handle.as_raw_fd())
    }

    #[cfg(unix)]
    pub fn take(handle: OwnedFd) -> OwnedFd {
        handle
    }

    #[cfg(unix)]
    pub fn optional_take(handle: Option<OwnedFd>) -> Option<OwnedFd> {
        handle
    }

    pub fn through_callback(callback: &impl HandleConsumer) -> usize {
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::fd::AsFd;
            callback.number(file.as_fd())
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsHandle;
            callback.number(file.as_handle())
        }
    }

    #[cfg(feature = "wide")]
    pub fn feature_width(&self, value: i64) -> i64 {
        value + 8
    }
    #[cfg(not(feature = "wide"))]
    pub fn feature_width(&self, value: i32) -> i32 {
        value + 4
    }

    // ------------------------------------------------------------------
    // Resource conversions. Every shape that the Unix descriptor methods
    // used to cover lives here, with a Windows counterpart chosen by cfg, so
    // the general test crates stay platform-agnostic.
    // ------------------------------------------------------------------

    /// Get the raw descriptor behind a borrowed file descriptor.
    #[cfg(unix)]
    pub fn fd_number(&self, fd: BorrowedFd<'_>) -> i32 {
        fd.as_raw_fd()
    }

    /// Get the raw handle behind a borrowed handle.
    #[cfg(windows)]
    pub fn fd_number(&self, handle: BorrowedHandle<'_>) -> isize {
        handle.as_raw_handle() as isize
    }

    /// Get the raw descriptor, or -1 if None.
    #[cfg(unix)]
    pub fn fd_number_optional(&self, fd: Option<BorrowedFd<'_>>) -> i32 {
        fd.map_or(-1, |f| f.as_raw_fd())
    }

    /// Get the raw handle, or 0 if None.
    #[cfg(windows)]
    pub fn fd_number_optional(&self, handle: Option<BorrowedHandle<'_>>) -> isize {
        handle.map_or(0, |h| h.as_raw_handle() as isize)
    }

    /// Maybe return a borrowed descriptor: < 0 error, 0 none, > 0 some.
    #[cfg(unix)]
    pub fn maybe_fd(&self, selector: i32) -> Result<Option<BorrowedFd<'_>>, ResourceError> {
        if selector < 0 {
            Err(ResourceError::InvalidInput())
        } else if selector == 0 {
            Ok(None)
        } else {
            // Safety: fd 0 (stdin) exists for the process lifetime
            Ok(Some(unsafe { BorrowedFd::borrow_raw(0) }))
        }
    }

    /// Maybe return a borrowed handle: < 0 error, 0 none, > 0 some.
    #[cfg(windows)]
    pub fn maybe_fd(&self, selector: i32) -> Result<Option<BorrowedHandle<'_>>, ResourceError> {
        if selector < 0 {
            Err(ResourceError::InvalidInput())
        } else if selector == 0 {
            Ok(None)
        } else {
            // Safety: the std handle stays open for the process lifetime.
            Ok(Some(unsafe {
                BorrowedHandle::borrow_raw(std::io::stdin().as_raw_handle())
            }))
        }
    }

    /// Duplicate a descriptor.
    #[cfg(unix)]
    pub fn dup_fd(&self, fd: BorrowedFd<'_>) -> OwnedFd {
        fd.try_clone_to_owned().expect("dup failed")
    }

    /// Duplicate a handle.
    #[cfg(windows)]
    pub fn dup_fd(&self, handle: BorrowedHandle<'_>) -> OwnedHandle {
        handle.try_clone_to_owned().expect("DuplicateHandle failed")
    }

    /// Duplicate an optional descriptor.
    #[cfg(unix)]
    pub fn maybe_dup_fd(
        &self,
        fd: Option<BorrowedFd<'_>>,
    ) -> Result<Option<OwnedFd>, ResourceError> {
        match fd {
            Some(f) => f
                .try_clone_to_owned()
                .map(Some)
                .map_err(|_| ResourceError::InvalidInput()),
            None => Ok(None),
        }
    }

    /// Duplicate an optional handle.
    #[cfg(windows)]
    pub fn maybe_dup_fd(
        &self,
        handle: Option<BorrowedHandle<'_>>,
    ) -> Result<Option<OwnedHandle>, ResourceError> {
        match handle {
            Some(h) => h
                .try_clone_to_owned()
                .map(Some)
                .map_err(|_| ResourceError::InvalidInput()),
            None => Ok(None),
        }
    }

    /// Optionally return an owned descriptor: < 0 error, 0 none, > 0 some.
    #[cfg(unix)]
    pub fn maybe_owned_fd(&self, selector: i32) -> Result<Option<OwnedFd>, ResourceError> {
        if selector < 0 {
            Err(ResourceError::InvalidInput())
        } else if selector == 0 {
            Ok(None)
        } else {
            // Safety: fd 0 (stdin) exists for the process lifetime
            let stdin = unsafe { BorrowedFd::borrow_raw(0) };
            stdin
                .try_clone_to_owned()
                .map(Some)
                .map_err(|_| ResourceError::InvalidInput())
        }
    }

    /// Optionally return an owned handle: < 0 error, 0 none, > 0 some.
    #[cfg(windows)]
    pub fn maybe_owned_fd(&self, selector: i32) -> Result<Option<OwnedHandle>, ResourceError> {
        if selector < 0 {
            Err(ResourceError::InvalidInput())
        } else if selector == 0 {
            Ok(None)
        } else {
            // Safety: the std handle stays open for the process lifetime.
            let stdin = unsafe { BorrowedHandle::borrow_raw(std::io::stdin().as_raw_handle()) };
            stdin
                .try_clone_to_owned()
                .map(Some)
                .map_err(|_| ResourceError::InvalidInput())
        }
    }

    pub fn through_feature_callback(callback: &impl FeatureConsumer) -> usize {
        callback.width(123) as usize
    }
}

#[ffier::export]
pub trait FeatureConsumer {
    #[cfg(feature = "wide")]
    #[ffier(index = 0)]
    fn width(&self, value: i64) -> i64 {
        value + 8
    }

    #[cfg(not(feature = "wide"))]
    #[ffier(index = 0)]
    fn width(&self, value: i32) -> i32 {
        value + 4
    }
}

#[ffier::export]
impl FeatureConsumer for HandleApi {
    #[cfg(feature = "wide")]
    fn width(&self, value: i64) -> i64 {
        value + 16
    }
}

#[ffier::export(cfg = "all(feature = \"owner-api\", feature = \"wide\")")]
impl FeatureConsumer for OptionalApi {
    fn width(&self, value: i64) -> i64 {
        value + 16
    }
}

#[ffier::export]
pub trait PlatformDefaults {
    #[cfg(all(unix, feature = "owner-api"))]
    #[ffier(index = 0)]
    fn platform_value(&self) -> usize {
        1
    }

    #[cfg(windows)]
    #[ffier(index = 0)]
    fn platform_value(&self) -> usize {
        11
    }
}

#[ffier::export(cfg = "feature = \"owner-api\"")]
impl PlatformDefaults for HandleApi {
    #[cfg(unix)]
    fn platform_value(&self) -> usize {
        77
    }

    #[cfg(windows)]
    fn platform_value(&self) -> usize {
        77
    }
}

#[ffier::export]
pub trait HandleConsumer {
    #[cfg(target_os = "windows")]
    #[ffier(index = 0)]
    fn number(&self, handle: BorrowedHandle<'_>) -> usize;

    #[cfg(unix)]
    #[ffier(index = 0)]
    fn number(&self, handle: BorrowedFd<'_>) -> usize {
        handle.as_raw_fd() as usize
    }

    #[cfg(target_os = "windows")]
    #[ffier(index = 1)]
    fn windows_only(&self) -> usize {
        11
    }
}

#[ffier::export(cfg = "unix")]
/// Duplicate a file descriptor.
pub fn clone_fd(fd: BorrowedFd<'_>) -> Result<OwnedFd, ResourceError> {
    fd.try_clone_to_owned()
        .map_err(|_| ResourceError::InvalidInput())
}

#[ffier::export(cfg = "windows")]
/// Duplicate a handle.
pub fn clone_fd(handle: BorrowedHandle<'_>) -> Result<OwnedHandle, ResourceError> {
    handle
        .try_clone_to_owned()
        .map_err(|_| ResourceError::InvalidInput())
}

ffier::library_definition!("platform_matrix", library_tag = 1,
    HandleApi = 1, trait HandleConsumer = 2, trait FeatureConsumer = 5,
    FeatureConsumer for HandleApi,
    #[cfg(all(feature = "owner-api", feature = "wide"))]
    FeatureConsumer for OptionalApi,
    trait PlatformDefaults = 7, PlatformDefaults for HandleApi,
    #[cfg(feature = "owner-api")] OptionalApi = 6,
    #[cfg(windows)] WindowsValue = 3,
    #[cfg(unix)] UnixValue = 4,
    #[cfg(windows)] fn windows_value,
    #[cfg(unix)] fn unix_value,
    ResourceError = 8,
    trait ffier_builtins::PushStr = 10,
    trait ffier_builtins::Error = 9,
    Error for ResourceError,
    fn resource,
    fn direct_resource,
    fn clone_fd,
    fn feature_width,
    fn conditional_export,
    fn generated_value,
);

ffier::generate_bridge!(
    local = __ffier_platform_matrix_metadata,
    schema_output = "../../target/ffier-platform-matrix.json"
);
