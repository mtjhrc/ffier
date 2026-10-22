//! expected-error: ffier explicit lifetimes on non-byte slice parameters are unsupported

#[ffier::export]
pub fn hold_text<'a>(_data: &[&'a str]) {}

fn main() {}
