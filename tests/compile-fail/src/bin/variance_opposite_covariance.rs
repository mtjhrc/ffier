//! expected-error: lifetime may not live long enough

// Declared covariance is compiler-checked against the native type.
// fn(&'a str) is contravariant, so the claimed coercion must fail.
struct Consumer<'a>(fn(&'a str));

#[ffier::export(covariant('a))]
impl<'a> Consumer<'a> {}

fn main() {}
