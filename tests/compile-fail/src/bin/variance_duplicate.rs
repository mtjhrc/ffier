//! expected-error: ffier lifetime variance must be declared exactly once

struct Duplicate<'a>(&'a str);

#[ffier::export(covariant('a), invariant('a))]
impl<'a> Duplicate<'a> {}

fn main() {}
