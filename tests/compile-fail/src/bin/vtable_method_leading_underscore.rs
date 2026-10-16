//! expected-error: exported function or method name `_process` starts with `_`, which is reserved for generated code

#[ffier::export]
pub trait Probe {
    #[ffier(index = 0)]
    fn _process(&self) -> u16;
}

fn main() {}
