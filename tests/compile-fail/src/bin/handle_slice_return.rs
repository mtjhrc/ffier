//! expected-error: ffier does not support slices of handle or other non-primitive types

pub struct Gadget;

pub struct Widget {
    gadgets: Vec<Gadget>,
}

#[ffier::export]
impl Widget {
    pub fn gadgets(&self) -> &[Gadget] {
        &self.gadgets
    }
}

fn main() {}
