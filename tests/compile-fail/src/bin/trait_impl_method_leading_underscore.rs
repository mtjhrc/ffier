//! expected-error: exported function or method name `_process` starts with `_`, which is reserved for generated code

pub trait Probe {
    fn _process(&self) -> u16;
}

pub struct Worker;

#[ffier::export]
impl Probe for Worker {
    fn _process(&self) -> u16 {
        0
    }
}

fn main() {}
