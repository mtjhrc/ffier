//! expected-error: ffier variance annotation names an unknown lifetime parameter

struct Unknown<'a>(&'a str);

#[ffier::export(covariant('b))]
impl<'a> Unknown<'a> {}

fn main() {}
