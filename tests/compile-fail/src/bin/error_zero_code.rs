//! expected-error: FfiError code 0 is reserved for success

#[derive(Debug, ffier::FfiError)]
#[non_exhaustive]
pub enum TestError {
    #[ffier(code = 0)]
    InvalidInput(),
}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid input")
    }
}

impl std::error::Error for TestError {}

fn main() {}
