//! expected-error: FFI type name `Never` is reserved

pub struct Never;

#[ffier::export]
impl Never {}

ffier::library_definition!("ui", library_tag = 1, Never = 1);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
