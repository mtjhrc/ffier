//! expected-error: requires #[ffier::export(send)] on its inherent impl

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

ffier::library_definition!(
    "ui",
    library_tag = 1,
    Counter = 1,
    trait ThreadBackend = 2,
    ThreadBackend for Counter,
);
ffier::generate_bridge!(local = __ffier_ui_metadata, schema_output = "/dev/null");

fn main() {}
