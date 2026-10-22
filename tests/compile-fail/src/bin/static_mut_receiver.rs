//! expected-error: ffier callback methods with 'static receivers are unsupported; use a named method lifetime or no_vtable

#[ffier::export]
pub trait StaticMutReceiver {
    #[ffier(index = 0)]
    fn reference(&'static mut self) -> &'static str;
}

fn main() {}
