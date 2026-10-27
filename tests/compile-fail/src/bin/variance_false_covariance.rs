//! expected-error: lifetime may not live long enough

// Declared covariance is compiler-checked against the native type.
// RefCell<&'a str> is invariant, so the claimed coercion must fail.
struct Invariant<'a>(std::cell::RefCell<&'a str>);

#[ffier::export(covariant('a))]
impl<'a> Invariant<'a> {}

fn main() {}
