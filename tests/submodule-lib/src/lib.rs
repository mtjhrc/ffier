//! Test crate that defines ffier types in submodules and references them
//! via `crate::` paths in `library_definition!`.
//!
//! This exercises the path-based resolution (the `pub use` alias trick)
//! to verify that types don't need to be at the crate root.

pub mod errors;
#[cfg(unix)]
pub mod fd;
pub mod nested;
pub mod types;

#[ffier::export]
#[repr(C)]
#[derive(Debug)]
pub struct Inner {
    pub value: u32,
}

// Re-export types at the crate root (as a normal library would)
pub use errors::SubError;
#[cfg(unix)]
#[ffier::reexport]
pub use fd::{probe_fd_source, FdSource};
#[ffier::reexport]
pub use nested::Outer;
#[ffier::reexport]
pub use types::{echo_scalars, Counter, Doubler};

// Use crate:: qualified paths — the whole point of this test.
// Previously this would have required manual `pub use types::_ffier_counter;` etc.
ffier::library_definition!(
    "subtest",
    library_tag = 2,
    crate::errors::SubError = 1,
    crate::types::Counter = 2,
    crate::types::Doubler = 3,
    trait ffier::builtins::PushStr = 4,
    trait ffier::builtins::Error = 5,
    Error for crate::errors::SubError,
    #[cfg(unix)]
    trait FdSource = 6,
    #[cfg(unix)]
    fn probe_fd_source,
    value Inner,
    value Outer,
    value crate::types::ScalarValues,
    fn crate::types::echo_scalars,
    #[cfg(feature = "extra")]
    value crate::types::ConfigValue,
);
