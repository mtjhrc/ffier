//! expected-error: ffier exported method lifetime bounds and where clauses are unsupported

#[ffier::export]
pub fn retain<'a: 'static>(_bytes: &'a [u8]) {}

fn main() {}
