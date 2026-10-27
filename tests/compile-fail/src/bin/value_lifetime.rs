//! expected-error: ffier value structs cannot be generic

#[ffier::export]
#[repr(C)]
pub struct Packet<'a> {
    pub kind: &'a u32,
}

ffier::library_definition!("ui", library_tag = 1, value Packet<'_>);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
