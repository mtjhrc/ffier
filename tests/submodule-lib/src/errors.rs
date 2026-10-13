#[derive(Debug, ffier::FfiError)]
#[non_exhaustive]
pub enum SubError {
    #[ffier(code = 1)]
    Oops,
    #[ffier(code = 2, message = "something went wrong")]
    BadThing(),
    #[ffier(code = 3)]
    #[non_exhaustive]
    Payload(i32, Box<str>, crate::types::Counter, crate::Inner),
}

impl std::fmt::Display for SubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubError::Oops => write!(f, "oops"),
            SubError::BadThing() => write!(f, "something went wrong"),
            SubError::Payload(number, message, counter, inner) => {
                write!(f, "{number}: {message}, {}, {}", counter.get(), inner.value)
            }
        }
    }
}

impl std::error::Error for SubError {}
