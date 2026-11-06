//! Registers the provider crate's exported trait from a *different* crate.
//!
//! Nothing here imports `BorrowedFd`, `Cell`, or the provider's private
//! dependencies, so every type in the registered API must resolve through the
//! definition-site aliases. Before those aliases existed for the standard OS
//! types, registering this trait here failed with
//! `cannot find type BorrowedFd in this scope`.

#![cfg(unix)]

pub use ffier_test_private_import_lib::{Handle, PrivateError, Source};

ffier::library_definition!(
    "private_import",
    library_tag = 3,
    ffier_test_private_import_lib::PrivateError = 1,
    Error for ffier_test_private_import_lib::PrivateError,
    ffier_test_private_import_lib::Handle<'_> = 2,
    trait ffier_test_private_import_lib::Source = 3,
    trait ffier_builtins::PushStr = 12,
    trait ffier_builtins::Error = 13,
);

ffier::generate_bridge!(
    local = __ffier_private_import_metadata,
    schema_output = "../../target/ffier-private-import.json"
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::{AsFd, BorrowedFd};

    struct Native;

    impl Source for Native {
        fn make<'a>(&'a self) -> Result<Handle<'a>, PrivateError> {
            Ok(Handle::new())
        }

        fn consume<'a>(&self, handle: Handle<'a>) -> Result<(), PrivateError> {
            assert!(handle.is_empty());
            Ok(())
        }

        fn echo_fd<'a>(&'a self, fd: BorrowedFd<'a>) -> Result<BorrowedFd<'a>, PrivateError> {
            Ok(fd)
        }

        fn bare_fd(&self, _fd: BorrowedFd<'_>) -> Result<(), PrivateError> {
            Ok(())
        }
    }

    /// The generated conversions must accept real (non-`'static`) lifetimes for
    /// an invariant handle in parameter and return position, and keep working
    /// for a privately imported `BorrowedFd` with and without an elided
    /// lifetime argument.
    #[test]
    fn private_imports_resolve_across_crates() {
        let native = Native;
        let file = std::fs::File::open("/dev/null").unwrap();
        let borrowed = file.as_fd();

        assert!(Source::consume(&native, Handle::new()).is_ok());
        assert!(Source::echo_fd(&native, borrowed).is_ok());
        assert!(Source::bare_fd(&native, borrowed).is_ok());
        let made = Source::make(&native).unwrap();
        assert!(made.is_empty());
    }
}
