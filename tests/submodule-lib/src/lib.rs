//! Test crate that defines ffier types in submodules and references them
//! via `crate::` paths in `library_definition!`.
//!
//! This exercises the path-based resolution (the `pub use` alias trick)
//! to verify that types don't need to be at the crate root.

pub mod errors;
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
    value Inner,
    value Outer,
    value crate::types::ScalarValues,
    fn crate::types::echo_scalars,
    #[cfg(feature = "extra")]
    value crate::types::ConfigValue,
);
