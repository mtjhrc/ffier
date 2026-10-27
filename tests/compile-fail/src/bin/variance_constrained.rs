//! expected-error: ffier opaque exports require only unconstrained lifetime parameters

struct Constrained<'a>(&'a str);

#[ffier::export(invariant('a))]
impl<'a: 'static> Constrained<'a> {}

fn main() {}
