trait Backend {}
pub struct Retained<'a>(&'a i32);
#[ffier::export]
pub fn bad(_backend: &impl Backend) -> Retained<'_> {
    unimplemented!()
}
fn main() {}
