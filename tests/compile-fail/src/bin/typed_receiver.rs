//! expected-error: ffier typed self parameters are unsupported; use self, &self, or &mut self

pub struct TypedReceiver;

#[ffier::export]
impl TypedReceiver {
    pub fn bytes<'a>(self: &'a Self) -> &'a [u8] {
        &[]
    }
}

fn main() {}
