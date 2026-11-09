//! expected-error: ffier: unsupported FFI type `*mut u8`

#[ffier::export]
pub fn write_byte(slot: *mut u8, value: u8) {
    unsafe { *slot = value }
}

ffier::library_definition!("ui", library_tag = 1, fn write_byte);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
