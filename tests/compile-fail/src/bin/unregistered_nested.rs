#[ffier::export]
#[repr(C)]
pub struct Inner {
    pub value: u32,
}

#[ffier::export]
#[repr(C)]
pub struct Outer {
    pub inner: Inner,
}

ffier::library_definition!("ui", library_tag = 1, value Outer);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "../../target/ui-unregistered.json");

fn main() {}
