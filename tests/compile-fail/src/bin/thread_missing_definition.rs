//! expected-error: requires a registered exported trait definition

pub struct Counter;

#[ffier::export]
impl Counter {}

#[ffier::export]
pub trait ThreadBackend: Send {
    #[ffier(index = 0)]
    fn step(&self) -> i32;
}

#[ffier::export]
impl ThreadBackend for Counter {
    fn step(&self) -> i32 {
        0
    }
}

#[ffier::export]
pub fn bad(_backend: impl ThreadBackend) {}

ffier::library_definition!(
    "ui",
    library_tag = 1,
    Counter = 1,
    ThreadBackend for Counter,
    fn bad,
);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
