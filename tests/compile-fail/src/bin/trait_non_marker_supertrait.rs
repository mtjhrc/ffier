//! expected-error: ffier exported traits support only Send/Sync supertraits

#[ffier::export]
pub trait Unsupported: std::fmt::Debug {}

fn main() {}
