//! expected-error: reserved vtable slot must leave room for the slot count to fit in u16, because the vtable header stores it as u16

#[ffier::export(reserved(65535))]
pub trait Probe {
    #[ffier(index = 0)]
    fn process(&self) -> i32;
}

fn main() {}
