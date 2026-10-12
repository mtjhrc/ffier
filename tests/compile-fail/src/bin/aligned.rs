#[ffier::export]
#[repr(C, align(16))]
pub struct Aligned {
    pub value: u32,
}

fn main() {}
