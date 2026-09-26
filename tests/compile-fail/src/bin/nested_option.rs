#[ffier::export]
#[repr(C)]
pub struct Value {
    pub value: u32,
}

#[ffier::export]
pub fn bad() -> Option<Option<Value>> {
    None
}

ffier::library_definition!("ui", library_tag = 1, value Value, fn bad);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "../../target/ui-nested-option.json");

fn main() {}
