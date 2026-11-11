//! expected-error: ffier borrowed Self returns require the `unstable_unsafe_borrowed_self` feature

#[ffier::export]
pub trait Widget {
    #[ffier(index = 0)]
    fn view(&mut self) -> Result<&mut Self, ()>;
}

fn main() {}
