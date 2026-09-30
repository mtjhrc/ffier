#[ffier::export]
pub trait Backend {
    #[ffier(index = 0)]
    fn value(&self) -> i32;
}
#[ffier::export]
pub fn bad<'a>(_backend: &'a impl Backend) {}
ffier::library_definition!("ui", library_tag = 1, trait Backend = 1, fn bad);
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "../../target/ui-callback-reference.json"
);
fn main() {}
