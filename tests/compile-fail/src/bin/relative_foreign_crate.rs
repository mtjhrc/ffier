//! expected-error: foreign crate path must start with `::`

pub mod other_lib {
    pub struct Foreign;
}

#[ffier::export]
pub fn read(#[ffier(foreign = other_lib, c_name = "OtherForeign")] _value: &other_lib::Foreign) {}

fn main() {}
