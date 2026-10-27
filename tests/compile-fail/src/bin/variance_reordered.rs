//! expected-error: ffier opaque exports require all lifetime parameters in declaration order, without specialized type arguments

struct Reordered<'a, 'b>(&'a str, &'b str);

#[ffier::export(covariant('a, 'b))]
impl<'a, 'b> Reordered<'b, 'a> {}

fn main() {}
