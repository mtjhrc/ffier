#![recursion_limit = "256"]

#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle, RawHandle};

pub struct HandleApi;

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

#[ffier::export]
impl HandleApi {
    #[cfg(windows)]
    pub fn value(_value: WindowsValue) -> usize {
        11
    }

    #[cfg(unix)]
    pub fn value(_value: UnixValue) -> usize {
        1
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
}

#[ffier::export]
pub trait HandleConsumer {
    #[cfg(target_os = "windows")]
    #[ffier(index = 0)]
    fn number(&self, handle: BorrowedHandle<'_>) -> usize {
        handle.as_raw_handle() as usize
    }

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

ffier::library_definition!("windows_handles", library_tag = 1,
    HandleApi = 1, trait HandleConsumer = 2,
    #[cfg(windows)] WindowsValue = 3,
    #[cfg(unix)] UnixValue = 4,
    #[cfg(windows)] fn windows_value,
    #[cfg(unix)] fn unix_value,
);

ffier::generate_bridge!(
    local = __ffier_windows_handles_metadata,
    schema_output = "../../target/ffier-windows-handles.json"
);
