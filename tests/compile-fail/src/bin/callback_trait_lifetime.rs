//! expected-error: ffier callback vtables with trait lifetime or type parameters are unsupported

#[ffier::export]
pub trait Backend<'a> {
    #[ffier(index = 0)]
    fn value(&self) -> i32;
}
fn main() {}
