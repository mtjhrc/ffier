pub use ffier_impl::*;
pub use ffier_rt::*;

// Handle allocation and destruction. Kept out of `ffier-rt` so a crate that
// only inspects or borrows handles does not pull in the allocator; the facade
// re-exports them so generated code keeps naming `ffier::`.
pub use ffier_bridge::{
    ffier_handle_consume, ffier_handle_drop, ffier_handle_new, ffier_handle_new_borrowed,
    ffier_object_array_free,
};

// Re-export built-in traits at the crate root so `ffier::Error`,
// `ffier::PushStr` work in user code and generated derive output.
pub use ffier_builtins::Error;
pub use ffier_builtins::PushStr;

/// Pre-annotated built-in traits (PushStr, etc.) ready for use in
/// `library_definition!` without redeclaring the trait signature.
pub mod builtins {
    pub use ffier_builtins::*;
}
