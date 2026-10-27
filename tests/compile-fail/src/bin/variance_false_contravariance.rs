//! expected-error: lifetime may not live long enough

// Declared contravariance is compiler-checked against the native type.
// RefCell<&'a str> is invariant, so the claimed coercion must fail.
struct Invariant<'a>(std::cell::RefCell<&'a str>);

#[ffier::export(contravariant('a))]
impl<'a> Invariant<'a> {}

fn main() {}
