#[derive(Debug, Clone, Copy, ffier::FfiError)]
pub enum SubError {
    #[ffier(code = 1)]
    Oops,
    #[ffier(code = 2, message = "something went wrong")]
    BadThing,
}

impl std::fmt::Display for SubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubError::Oops => write!(f, "oops"),
            SubError::BadThing => write!(f, "something went wrong"),
        }
    }
}

impl std::error::Error for SubError {}
