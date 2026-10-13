//! expected-error: FfiError enums must be #[non_exhaustive]

#[derive(ffier::FfiError)]
pub enum TestError {
    #[ffier(code = 1)]
    InvalidInput(),
}

fn main() {}
