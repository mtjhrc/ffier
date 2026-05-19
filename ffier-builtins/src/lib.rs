//! Pre-annotated ffier traits for built-in runtime types.
//!
//! This crate applies `#[ffier_annotations::implementable(foreign)]` to traits
//! defined in `ffier-rt`, generating the vtable struct and metadata macro so
//! downstream crates can register them in their `library_definition!` without
//! redeclaring the trait signature:
//!
//! ```ignore
//! ffier::library_definition!("mylib",
//!     trait ffier::builtins::PushStr = 10,
//! );
//! ```

// Re-export ffier-rt so generated code that references `ffier::` paths
// (from the #[implementable] proc macro) can resolve them. The proc macro
// emits `ffier::VtableHandle`, `ffier::FfiType`, etc. — we provide `ffier`
// as an alias for `ffier_rt`.
use ffier_rt as ffier;

pub use ffier_rt::PushStr;

#[ffier_annotations::implementable(foreign)]
trait PushStr {
    #[ffier(index = 0)]
    fn push(&mut self, s: &str) -> bool;
}
