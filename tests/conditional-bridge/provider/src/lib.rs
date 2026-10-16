//! Keep Widget's metadata available while its impl and FFI helpers are gated.

pub mod types {
    // Export metadata intentionally preserves definition-site crate:: paths.
    #![allow(clippy::crate_in_macro_def)]

    pub type Value = i32;

    pub struct Widget {
        pub value: Value,
    }

    #[ffier::export(cfg = "feature = \"a_feature\"")]
    impl Widget {
        pub fn new(value: crate::types::Value) -> Self {
            Self { value }
        }

        pub fn get(&self) -> self::Value {
            self.value
        }
    }
}

#[ffier::reexport]
pub use types::Widget;
