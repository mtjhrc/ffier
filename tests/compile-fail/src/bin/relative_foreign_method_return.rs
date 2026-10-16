//! expected-error: foreign crate path must start with `::`

pub mod other_lib {
    pub struct Foreign;
}

pub struct Factory;

#[ffier::export]
impl Factory {
    #[ffier(foreign_return = other_lib, c_name = "OtherForeign")]
    pub fn create() -> other_lib::Foreign {
        other_lib::Foreign
    }
}

fn main() {}
