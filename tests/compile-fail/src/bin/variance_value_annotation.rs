//! expected-error: ffier lifetime variance annotations are supported only on inherent impls

#[ffier::export(covariant('a))]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Value {
    pub number: u32,
}

fn main() {}
