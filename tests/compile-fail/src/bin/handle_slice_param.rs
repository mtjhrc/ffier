//! expected-error: ffier does not support slices of handle or other non-primitive types

pub struct Gadget;

#[ffier::export]
pub fn sum_gadgets(_gadgets: &[&Gadget]) -> i32 {
    0
}

fn main() {}
