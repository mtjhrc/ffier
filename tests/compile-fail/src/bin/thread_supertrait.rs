//! expected-error: ffier exported traits support only Send/Sync supertraits

#[ffier::export]
pub trait Unsupported: Send + std::fmt::Debug {}

fn main() {}
