//! expected-error: #[ffier(returns_self)] on `replace` needs the return type `Self` (or `Result<Self, E>`), matching its receiver

pub struct View<'a>(&'a str);

#[ffier::export(covariant('a))]
impl<'a> View<'a> {
    #[ffier(returns_self)]
    pub fn replace<'b>(self, text: &'b str) -> View<'b> {
        let _ = self.0;
        View(text)
    }
}

fn main() {}
