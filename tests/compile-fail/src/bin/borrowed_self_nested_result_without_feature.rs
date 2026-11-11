//! expected-error: ffier borrowed Self returns require the `unstable_unsafe_borrowed_self` feature

pub struct Widget;

#[ffier::export]
impl Widget {
    pub fn view(&self) -> Result<Result<&Self, ()>, ()> {
        Ok(Ok(self))
    }
}

fn main() {}
