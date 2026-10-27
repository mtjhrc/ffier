//! expected-error: has unsupported type `Box`

#[ffier::export]
#[repr(C)]
pub struct Packet {
    pub kind: Box<u32>,
}

ffier::library_definition!("ui", library_tag = 1, value Packet);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
