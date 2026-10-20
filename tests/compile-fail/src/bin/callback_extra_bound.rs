//! expected-error: ffier impl Trait supports one trait bound and lifetime bounds

trait Backend {}
#[ffier::export]
pub fn bad(_backend: impl Backend + Send) {}
fn main() {}
