//! expected-error: ffier opaque exports require all lifetime parameters in declaration order, without specialized type arguments

struct Borrowed<'a>(&'a i32);

#[ffier::export(send)]
impl Borrowed<'static> {}

fn main() {}
