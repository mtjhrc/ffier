#[derive(Debug, Clone, Copy, ffier::FfiError)]
pub enum SubError {
    #[ffier(code = 1)]
    Oops,
    #[ffier(code = 2, message = "something went wrong")]
    BadThing,
}
