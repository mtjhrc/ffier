// Keep these imports private to exercise definition-site OS type resolution.
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};

use crate::SubError;

#[ffier::export]
pub trait FdSource: Send {
    #[ffier(index = 0)]
    fn read_notify_fd(&self) -> Result<BorrowedFd<'_>, SubError>;

    #[ffier(index = 1)]
    fn clone_fd(&self, fd: BorrowedFd<'_>) -> Result<OwnedFd, SubError>;

    #[ffier(index = 2)]
    fn preserve_fd(&self, fd: OwnedFd) -> OwnedFd;

    #[ffier(index = 3)]
    fn raw_fd(&self, fd: RawFd) -> RawFd;

    #[ffier(index = 4)]
    fn borrowed_fd<'a>(&self, fd: BorrowedFd<'a>) -> Option<BorrowedFd<'a>>;
}

#[ffier::export]
pub fn probe_fd_source(
    #[ffier(dispatch = vtable)] source: &mut impl FdSource,
) -> Result<bool, SubError> {
    let fd = source.read_notify_fd()?;
    let raw = fd.as_raw_fd();
    let duplicate = source.clone_fd(fd)?;
    drop(source.preserve_fd(duplicate));
    Ok(source.raw_fd(raw) == raw && source.borrowed_fd(fd).unwrap().as_raw_fd() == raw)
}
