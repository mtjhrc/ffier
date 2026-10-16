//! expected-error: exported function or method name `_process` starts with `_`, which is reserved for generated code

#[ffier::export]
pub fn _process() -> u16 {
    0
}

fn main() {}
