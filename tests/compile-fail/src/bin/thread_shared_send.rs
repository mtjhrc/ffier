//! expected-error: requires Sync on Send trait

#[ffier::export]
pub trait SendBackend: Send {
    #[ffier(index = 0)]
    fn step(&self) -> i32;
}

#[ffier::export]
pub fn bad(#[ffier(dispatch = vtable)] _backend: &impl SendBackend) {}

ffier::library_definition!("ui", library_tag = 1, trait SendBackend = 1, fn bad);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
