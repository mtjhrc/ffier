//! expected-error: ffier send/sync annotations require exporting the type for all its lifetime parameters

struct Borrowed<'a>(&'a i32);

#[ffier::export(send)]
impl Borrowed<'static> {}

fn main() {}
