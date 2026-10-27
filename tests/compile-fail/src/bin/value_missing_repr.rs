//! expected-error: ffier value structs require #[repr(C)]

#[ffier::export]
pub struct Packet {
    pub kind: u32,
}

ffier::library_definition!("ui", library_tag = 1, value Packet);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
