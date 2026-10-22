//! expected-error: ffier explicit lifetimes on non-byte slice parameters are unsupported

#[ffier::export]
pub fn hold<'a>(_data: &'a [u32]) {}

fn main() {}
