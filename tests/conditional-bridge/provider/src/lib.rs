//! Keep Widget's metadata available while its impl and FFI helpers are gated.

pub mod types {
    // Export metadata intentionally preserves definition-site crate:: paths.
    #![allow(clippy::crate_in_macro_def)]

    // Register signature types so schema references resolve in binding generators.
    #[ffier::export]
    #[repr(i32)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Mode {
        Idle = 0,
        Active = 1,
    }

    pub struct Widget {
        pub mode: Mode,
    }

    #[ffier::export(cfg = "feature = \"a_feature\"")]
    impl Widget {
        pub fn new(mode: crate::types::Mode) -> Self {
            Self { mode }
        }

        pub fn get(&self) -> self::Mode {
            self.mode
        }
    }
}

#[ffier::reexport]
pub use types::Mode;
#[ffier::reexport]
pub use types::Widget;
