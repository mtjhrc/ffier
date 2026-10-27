//! expected-error: ffier send/sync annotations are supported only on inherent impls

#[ffier::export(send)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Value {
    pub number: u32,
}

fn main() {}
