#![recursion_limit = "256"]

#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, BorrowedHandle, OwnedHandle, RawHandle};

#[cfg(windows)]
pub struct HandleApi;

#[cfg(windows)]
#[ffier::export]
impl HandleApi {
    pub fn raw(handle: BorrowedHandle<'_>) -> RawHandle {
        handle.as_raw_handle()
    }

    pub fn optional_raw(handle: Option<BorrowedHandle<'_>>) -> RawHandle {
        handle.map_or(core::ptr::null_mut(), |handle| handle.as_raw_handle())
    }

    pub fn take(handle: OwnedHandle) -> OwnedHandle {
        handle
    }

    pub fn optional_take(handle: Option<OwnedHandle>) -> Option<OwnedHandle> {
        handle
    }
}

#[cfg(windows)]
ffier::library_definition!("windows_handles", library_tag = 1, HandleApi = 1,);

#[cfg(windows)]
ffier::generate_bridge!(
    local = __ffier_windows_handles_metadata,
    schema_output = "../../target/ffier-windows-handles.json"
);
