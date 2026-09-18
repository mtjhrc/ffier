#[ffier::export]
#[repr(C)]
pub struct UnsupportedField {
    pub value: String,
}

ffier::library_definition!("ui", library_tag = 1, value UnsupportedField);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "../../target/ui-unsupported.json");

fn main() {}
