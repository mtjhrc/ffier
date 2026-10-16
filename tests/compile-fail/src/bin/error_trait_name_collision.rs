//! expected-error: FFI type name `Error` is already registered

#[derive(Debug, ffier::FfiError)]
#[non_exhaustive]
pub enum Error {
    #[ffier(code = 1)]
    Failed(),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("failure")
    }
}

impl std::error::Error for Error {}

ffier::library_definition!(
    "ui",
    library_tag = 1,
    Error = 1,
    trait ffier::builtins::Error = 2,
    trait ffier::builtins::PushStr = 3,
    Error for Error,
);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
