//! expected-error: ffier: `Lender::lend` returns the borrowed handle `&Gadget`, which is not supported

pub struct Gadget;
#[ffier::export]
impl Gadget {}

#[ffier::export]
pub trait Lender {
    #[ffier(index = 0)]
    fn lend(&self) -> &Gadget;
}
ffier::library_definition!("ui", library_tag = 1, Gadget = 1, trait Lender = 2);
fn main() {}
