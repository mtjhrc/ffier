//! expected-error: ffier variance groups require at least one lifetime

struct Empty<'a>(&'a str);

#[ffier::export(covariant())]
impl<'a> Empty<'a> {}

fn main() {}
