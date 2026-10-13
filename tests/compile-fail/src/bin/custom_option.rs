//! expected-error: FFI type name `Option` is reserved

pub mod custom {
    pub struct Option<T>(pub T);
}

#[ffier::export]
pub fn read(value: custom::Option<&str>) -> usize {
    value.0.len()
}

ffier::library_definition!("ui", library_tag = 1, fn read);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
