use crate::{FfiBorrow, FfiType};

#[derive(Debug, Clone, Copy, ffier::FfiError)]
#[non_exhaustive]
pub enum SubError {
    #[ffier(code = 1)]
    Oops,
    #[ffier(code = 2, message = "something went wrong")]
    BadThing(),
    #[ffier(code = 3)]
    Mixed(i32, #[ffier(opaque)] (), u64),
}

impl std::fmt::Display for SubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubError::Oops => write!(f, "oops"),
            SubError::BadThing() => write!(f, "something went wrong"),
            SubError::Mixed(first, (), last) => write!(f, "mixed: {first}, {last}"),
        }
    }
}

impl std::error::Error for SubError {}
