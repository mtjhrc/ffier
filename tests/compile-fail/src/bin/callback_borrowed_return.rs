//! expected-error: ffier borrowed callbacks with lifetime-bearing returns are unsupported

trait Backend {}
pub struct Retained<'a>(&'a i32);
#[ffier::export]
pub fn bad(_backend: &impl Backend) -> Retained<'_> {
    unimplemented!()
}
fn main() {}
