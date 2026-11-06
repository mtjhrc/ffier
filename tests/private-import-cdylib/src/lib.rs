//! Registers the provider crate's exported trait from a *different* crate.
//!
//! Nothing here imports `BorrowedFd`, `Cell`, or the provider's private
//! dependencies, so every type in the registered API must resolve through the
//! definition-site aliases. Before those aliases existed for the standard OS
//! types, merely compiling this crate failed with
//! `cannot find type BorrowedFd in this scope` and
//! `lifetime may not live long enough`, because the generated vtable impl is
//! emitted here and pinned those types to `'static`.
//!
//! The generated `VtableSource` impl therefore has to accept non-`'static`
//! lifetimes for an invariant handle and for a privately imported
//! `BorrowedFd`. Implementing the provider trait with matching signatures below
//! forces both the registration and the generated conversions to compile.

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

    /// Round-trip a handle and a descriptor at a lifetime bounded by a local,
    /// so the conversions cannot be satisfied by `'static`.
    #[test]
    fn private_imports_resolve_across_crates() {
        let native = Native;
        let file = std::fs::File::open("/dev/null").unwrap();

        {
            let borrowed = file.as_fd();
            assert!(Source::echo_fd(&native, borrowed).is_ok());
            assert!(Source::bare_fd(&native, borrowed).is_ok());
        }

        {
            let handle = Handle::new();
            assert!(Source::consume(&native, handle).is_ok());
        }

        let made = Source::make(&native).unwrap();
        assert!(made.is_empty());
    }

    /// The generated vtable wrapper must expose the same API. Referencing its
    /// impl forces the generated conversions to be compiled against these
    /// concrete, short lifetimes.
    #[test]
    fn generated_vtable_impl_typechecks_with_short_lifetimes() {
        fn take<'a>(
            wrapper: &VtableSource<'a>,
            handle: Handle<'a>,
            fd: BorrowedFd<'a>,
        ) -> Result<(), PrivateError> {
            // Not called: the vtable slots are unset. Compiling this body is
            // what proves the generated impl accepts a non-'static lifetime.
            if handle.is_empty() {
                let _ = Source::echo_fd(wrapper, fd)?;
                let _ = Source::consume(wrapper, handle)?;
            }
            Ok(())
        }

        // Force the body to be typechecked without calling it (the vtable
        // slots are unset, so calling would abort).
        let _ = take;
    }
}
