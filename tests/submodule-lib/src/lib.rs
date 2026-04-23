//! Test crate that defines ffier types in submodules and references them
//! via `crate::` paths in `library_definition!`.
//!
//! This exercises the path-based resolution (the `pub use` alias trick)
//! to verify that types don't need to be at the crate root.

pub mod errors;
pub mod types;

// Re-export types at the crate root (as a normal library would)
pub use errors::SubError;
pub use types::{Counter, Doubler};

// Use crate:: qualified paths — the whole point of this test.
// Previously this would have required manual `pub use types::_ffier_counter;` etc.
ffier::library_definition!(
    "subtest",
    crate::errors::SubError = 1,
    crate::types::Counter = 2,
    crate::types::Doubler = 3,
);
