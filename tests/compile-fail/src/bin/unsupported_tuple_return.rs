//! expected-error: ffier: unsupported FFI type `(u8, u8)`

#[ffier::export]
pub fn split_pair(value: u8) -> (u8, u8) {
    (value, value)
}

ffier::library_definition!("ui", library_tag = 1, fn split_pair);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
