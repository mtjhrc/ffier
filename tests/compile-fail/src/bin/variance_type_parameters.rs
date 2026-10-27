//! expected-error: ffier opaque exports require only unconstrained lifetime parameters

struct Generic<'a, T>(&'a T);

#[ffier::export(covariant('a))]
impl<'a, T> Generic<'a, T> {}

fn main() {}
