//! expected-error: `FfiType` is not implemented for `Foo

#![allow(dead_code)]

// This root type is invariant and has no exported variance contract.
pub struct Foo<'a>(std::cell::RefCell<&'a str>);

pub mod other {
    pub struct Foo<'a>(pub &'a str);
}

// A qualified inherent export can verify other::Foo while registration and
// bridge metadata resolve Foo at the crate root. The generated plumbing must
// be bound to the verified native type, not the same-named root type.
#[ffier::export(covariant('a))]
impl<'a> other::Foo<'a> {}

#[ffier::export]
pub fn create<'a>(text: &'a str) -> Foo<'a> {
    Foo(std::cell::RefCell::new(text))
}

#[ffier::export]
pub fn replace<'a>(target: &Foo<'a>, text: &'a str) {
    *target.0.borrow_mut() = text;
}

#[ffier::export]
pub fn read<'a>(target: &'a Foo<'_>) -> &'a str {
    *target.0.borrow()
}

ffier::library_definition!("identity_fail", library_tag = 9,
    Foo<'_> = 1, fn create, fn replace, fn read);
ffier::generate_bridge!(
    local = __ffier_identity_fail_metadata,
    schema_output = "/dev/null"
);

fn main() {}
