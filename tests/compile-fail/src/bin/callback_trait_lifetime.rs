#[ffier::export]
pub trait Backend<'a> {
    #[ffier(index = 0)]
    fn value(&self) -> i32;
}
fn main() {}
