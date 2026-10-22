//! expected-error: ffier send/sync annotations require an inherent impl with only unconstrained lifetime parameters

struct Borrowed<'a>(&'a i32);

#[ffier::export(send)]
impl<'a: 'static> Borrowed<'a> {}

fn main() {}
