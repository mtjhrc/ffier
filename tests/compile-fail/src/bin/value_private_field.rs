//! expected-error: ffier value-struct fields must be public

pub mod inner {
    #[ffier::export]
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Packet {
        kind: u32,
    }

    ffier::library_definition!("ui", library_tag = 1, value Packet);
    ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");
}

fn main() {}
