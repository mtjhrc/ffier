//! expected-error: ffier impl Trait supports one trait bound and lifetime bounds

trait Backend<'a> {}
#[ffier::export]
pub fn bad(_backend: impl for<'a> Backend<'a>) {}
fn main() {}
