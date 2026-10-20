//! expected-error: ffier thread-safe traits support only explicit Send/Sync supertraits

#[ffier::export]
pub trait Unsupported: Send + std::fmt::Debug {}

fn main() {}
