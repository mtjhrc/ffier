//! expected-error: shared vtable dispatch for param

#[ffier::export]
pub trait Backend {
    #[ffier(index = 0)]
    fn value(&mut self) -> i32;
}
#[ffier::export]
pub fn bad(#[ffier(dispatch = vtable)] _backend: &impl Backend) {}
ffier::library_definition!("ui", library_tag = 1, trait Backend = 1, fn bad);
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "/dev/null"
);
fn main() {}
