//! expected-error: vtable index must leave room for the slot count to fit in u16, because the vtable header stores it as u16

#[ffier::export]
pub trait Probe {
    #[ffier(index = 65535)]
    fn process(&self) -> i32;
}

fn main() {}
