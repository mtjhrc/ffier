//! Exports a trait whose types are only imported privately.
//!
//! Nothing here registers the API: `tests/private-import-cdylib` does that, so
//! every type name in the registered surface has to resolve through the
//! definition-site aliases ffier emits next to these definitions. This is the
//! shape that broke when the standard OS types were special-cased instead of
//! aliased.

#![cfg(unix)]

use std::cell::Cell;
use std::os::fd::BorrowedFd;

use thiserror::Error;

#[derive(Error, Debug, ffier::FfiError)]
#[non_exhaustive]
pub enum PrivateError {
    #[error("failed")]
    #[ffier(code = 1)]
    Failed,
}

/// Invariant in `'a`: `Cell<&'a ()>` is invariant, so a `'static`
/// instantiation cannot silently stand in for a shorter borrow.
pub struct Handle<'a>(Cell<Option<&'a ()>>);

impl Default for Handle<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[ffier::export(invariant('a))]
impl<'a> Handle<'a> {
    pub fn new() -> Self {
        Handle(Cell::new(None))
    }

    pub fn is_empty(&self) -> bool {
        self.0.get().is_none()
    }
}

#[ffier::export]
pub trait Source: Send {
    /// Return an invariant handle through the out-param convention.
    #[ffier(index = 0)]
    fn make<'a>(&'a self) -> Result<Handle<'a>, PrivateError>;

    /// Accept an invariant handle by value.
    #[ffier(index = 1)]
    fn consume<'a>(&self, handle: Handle<'a>) -> Result<(), PrivateError>;

    /// `BorrowedFd` is imported privately above; both directions.
    #[ffier(index = 2)]
    fn echo_fd<'a>(&'a self, fd: BorrowedFd<'a>) -> Result<BorrowedFd<'a>, PrivateError>;

    /// An omitted lifetime argument must keep working.
    #[ffier(index = 3)]
    fn bare_fd(&self, fd: BorrowedFd) -> Result<(), PrivateError>;
}
