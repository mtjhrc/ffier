//! expected-error: ffier: unsupported FFI type `(u8, u8)`

#[ffier::export]
pub fn sum_pair(pair: (u8, u8)) -> u8 {
    pair.0 + pair.1
}

ffier::library_definition!("ui", library_tag = 1, fn sum_pair);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
