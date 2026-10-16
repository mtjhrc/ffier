//! expected-error: exported function or method name `_process` starts with `_`, which is reserved for generated code

pub struct Probe;

#[ffier::export]
impl Probe {
    pub fn _process(&self) -> u16 {
        0
    }
}

fn main() {}
