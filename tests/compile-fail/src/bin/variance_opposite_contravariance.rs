//! expected-error: lifetime may not live long enough

// Declared contravariance is compiler-checked against the native type.
// &'a str is covariant, so the claimed coercion must fail.
struct Borrowed<'a>(&'a str);

#[ffier::export(contravariant('a))]
impl<'a> Borrowed<'a> {}

fn main() {}
