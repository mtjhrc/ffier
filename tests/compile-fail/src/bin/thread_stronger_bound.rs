//! expected-error: stronger use-site Send/Sync bounds for callback

#[ffier::export]
pub trait Backend {
    #[ffier(index = 0)]
    fn step(&self) -> i32;
}

#[ffier::export]
pub fn bad(#[ffier(dispatch = vtable)] _backend: impl Backend + Send) {}

ffier::library_definition!("ui", library_tag = 1, trait Backend = 1, fn bad);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
