//! expected-error: #[ffier(returns_self)] on `bump` is only supported on methods of an exported inherent impl

pub struct Counter(i32);
#[ffier::export]
impl Counter {}

#[ffier::export(no_vtable)]
pub trait Bump {
    #[ffier(index = 0)]
    fn bump(&mut self);
}

#[ffier::export]
impl Bump for Counter {
    #[ffier(returns_self)]
    fn bump(&mut self) {
        self.0 += 1;
    }
}
ffier::library_definition!("ui", library_tag = 1, Counter = 1, trait Bump = 2, Bump for Counter);
fn main() {}
