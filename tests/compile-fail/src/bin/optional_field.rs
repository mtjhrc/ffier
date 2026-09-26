#[ffier::export]
#[repr(C)]
pub struct OptionalField {
    pub value: Option<u32>,
}

fn main() {}
