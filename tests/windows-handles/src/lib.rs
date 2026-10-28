#![recursion_limit = "256"]

#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle, RawHandle};

#[derive(Debug, ffier::FfiError)]
#[non_exhaustive]
pub enum PlatformError {
    #[ffier(code = 1)]
    Unavailable,
}
impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("platform resource unavailable")
    }
}
impl std::error::Error for PlatformError {}

pub struct HandleApi;

#[ffier::export]
impl HandleApi {
    #[cfg(windows)]
    pub fn raw(handle: BorrowedHandle<'_>) -> RawHandle {
        handle.as_raw_handle()
    }

    #[cfg(windows)]
    pub fn optional_raw(handle: Option<BorrowedHandle<'_>>) -> RawHandle {
        handle.map_or(core::ptr::null_mut(), |handle| handle.as_raw_handle())
    }

    #[cfg(windows)]
    pub fn take(handle: OwnedHandle) -> OwnedHandle {
        handle
    }

    #[cfg(windows)]
    pub fn optional_take(handle: Option<OwnedHandle>) -> Option<OwnedHandle> {
        handle
    }

    #[cfg(windows)]
    pub fn from_ready(source: &impl ReadySource) -> RawHandle {
        assert!(source.ready_handle_is_valid());
        source.ready_handle().as_raw_handle()
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

    #[cfg(unix)]
    pub fn from_ready(source: &impl ReadySource) -> RawFd {
        assert!(source.ready_fd_is_valid());
        source.ready_fd().as_raw_fd()
    }
}

/// Both libkrun logging signatures are retained in one portable schema.
#[ffier::export]
pub mod logging {
    #[cfg(unix)]
    pub fn init_log(handle: Option<std::os::fd::BorrowedFd<'static>>) -> bool {
        handle.is_some()
    }

    #[cfg(windows)]
    pub fn init_log(handle: Option<std::os::windows::io::BorrowedHandle<'static>>) -> bool {
        handle.is_some()
    }
}

pub struct ConsoleBuilder;

#[ffier::export]
impl ConsoleBuilder {
    pub fn new() -> Self {
        Self
    }

    #[cfg(unix)]
    pub fn add_tty_port(&self, tty_fd: BorrowedFd<'_>) -> Result<RawFd, PlatformError> {
        Ok(tty_fd.as_raw_fd())
    }

    #[cfg(windows)]
    pub fn add_tty_port(&self, tty_fd: BorrowedHandle<'_>) -> Result<RawHandle, PlatformError> {
        Ok(tty_fd.as_raw_handle())
    }
}

impl Default for ConsoleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[ffier::export]
pub trait ReadySource {
    #[cfg(unix)]
    #[ffier(index = 0)]
    fn ready_fd(&self) -> BorrowedFd<'_>;

    #[cfg(windows)]
    #[ffier(index = 1)]
    fn ready_handle(&self) -> BorrowedHandle<'_>;

    #[cfg(unix)]
    #[ffier(index = 2)]
    fn ready_fd_is_valid(&self) -> bool {
        self.ready_fd().as_raw_fd() >= 0
    }

    #[cfg(windows)]
    #[ffier(index = 3)]
    fn ready_handle_is_valid(&self) -> bool {
        !self.ready_handle().as_raw_handle().is_null()
    }
}

ffier::library_definition!(
    "windows_handles",
    library_tag = 1,
    HandleApi = 1,
    ConsoleBuilder = 2,
    trait ReadySource = 3,
    PlatformError = 4,
    trait ffier::builtins::PushStr = 5,
    trait ffier::builtins::Error = 6,
    Error for PlatformError,
    fn logging::init_log,
);

ffier::generate_bridge!(
    local = __ffier_windows_handles_metadata,
    schema_output = "../../target/ffier-windows-handles.json"
);
