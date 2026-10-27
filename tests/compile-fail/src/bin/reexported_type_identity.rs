//! expected-error: `FfiType` is not implemented for `Foo

#![allow(dead_code)]

pub struct Foo<'a>(std::cell::RefCell<&'a str>);

pub mod other {
    pub struct Foo<'a>(pub &'a str);

    #[ffier::export(send, sync)]
    impl<'a> Foo<'a> {}
}

// Forward only the checked type's metadata and helpers, never root Foo.
pub use other::{__ffier_meta_Foo, _ffier_foo};

#[ffier::export]
pub fn create<'a>(text: &'a str) -> Foo<'a> {
    Foo(std::cell::RefCell::new(text))
}

#[ffier::export]
pub fn replace<'a>(target: &Foo<'a>, text: &'a str) {
    *target.0.borrow_mut() = text;
}

ffier::library_definition!("identity_reexport", library_tag = 9,
    Foo<'_> = 1, fn create, fn replace);
ffier::generate_bridge!(
    local = __ffier_identity_reexport_metadata,
    schema_output = "/dev/null"
);

fn main() {}
