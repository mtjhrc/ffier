//! expected-error: ffier opaque exports require all lifetime parameters in declaration order, without specialized type arguments

struct Specialized<'a>(&'a str);

#[ffier::export(invariant('a))]
impl Specialized<'static> {}

fn main() {}
