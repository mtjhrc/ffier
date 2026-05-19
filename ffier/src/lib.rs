pub use ffier_annotations::*;
pub use ffier_rt::*;

/// Pre-annotated built-in traits (PushStr, etc.) ready for use in
/// `library_definition!` without redeclaring the trait signature.
pub mod builtins {
    pub use ffier_builtins::*;
}
