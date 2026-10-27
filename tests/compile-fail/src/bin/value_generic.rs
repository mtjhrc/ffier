//! expected-error: ffier value structs cannot be generic

#[ffier::export]
#[repr(C)]
pub struct Packet<T> {
    pub kind: T,
}

ffier::library_definition!("ui", library_tag = 1, value Packet<u32>);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
