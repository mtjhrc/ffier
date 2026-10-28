// The notification source mirrors libkrun input's borrowed readiness FD;
// Windows uses a HANDLE while both contracts remain in one schema.
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
#[cfg(windows)]
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle};

#[cfg_attr(feature = "ffi", ffier::export)]
pub trait ReadySource {
    #[cfg(unix)]
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn ready_fd(&self) -> BorrowedFd<'_>;

    #[cfg(windows)]
    #[cfg_attr(feature = "ffi", ffier(index = 1))]
    fn ready_handle(&self) -> BorrowedHandle<'_>;

    #[cfg(unix)]
    #[cfg_attr(feature = "ffi", ffier(index = 2))]
    fn ready_fd_is_valid(&self) -> bool {
        self.ready_fd().as_raw_fd() >= 0
    }

    #[cfg(windows)]
    #[cfg_attr(feature = "ffi", ffier(index = 3))]
    fn ready_handle_is_valid(&self) -> bool {
        !self.ready_handle().as_raw_handle().is_null()
    }
}

pub struct ReadyFile(pub std::fs::File);
impl ReadySource for ReadyFile {
    #[cfg(unix)]
    fn ready_fd(&self) -> BorrowedFd<'_> { self.0.as_fd() }
    #[cfg(windows)]
    fn ready_handle(&self) -> BorrowedHandle<'_> { self.0.as_handle() }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    #[test]
    fn readiness_resource_stays_borrowed_from_its_owner() {
        let source = ReadyFile(std::fs::File::open(std::env::current_exe().unwrap()).unwrap());
        #[cfg(unix)]
        assert!(source.ready_fd_is_valid());
        #[cfg(windows)]
        assert!(source.ready_handle_is_valid());
        assert!(source.0.metadata().is_ok());
    }
}

// A method-only platform API must keep its own native type alias unconditional.
pub struct PlatformApi;
#[cfg_attr(feature = "ffi", ffier::export)]
impl PlatformApi {
    #[cfg(unix)]
    pub fn fd_number(fd: BorrowedFd<'_>) -> std::os::fd::RawFd { fd.as_raw_fd() }

    #[cfg(windows)]
    pub fn handle_number(handle: BorrowedHandle<'_>) -> std::os::windows::io::RawHandle {
        handle.as_raw_handle()
    }
}
