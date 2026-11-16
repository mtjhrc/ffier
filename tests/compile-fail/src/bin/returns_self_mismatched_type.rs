//! expected-error: #[ffier(returns_self)] on `other` needs the return type `&Self` (or `Result<&Self, E>`), matching its receiver

mod owner {
    pub struct Item(pub(super) super::other::Item);
}

mod other {
    pub struct Item;
}

#[ffier::export]
impl owner::Item {
    #[ffier(returns_self)]
    pub fn other(&self) -> &other::Item {
        &self.0
    }
}

fn main() {}
