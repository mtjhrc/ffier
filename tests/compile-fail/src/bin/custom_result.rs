//! expected-error: FFI type name `Result` is reserved

pub mod custom {
    pub struct Result<T, E>(pub T, pub E);
}

#[ffier::export]
pub fn read() -> custom::Result<std::option::Option<&'static str>, u8> {
    custom::Result(Some("custom"), 0)
}

ffier::library_definition!("ui", library_tag = 1, fn read);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
