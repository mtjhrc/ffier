//! expected-error: ffier module exports require an inline module

#[ffier::export]
#[path = "../external_module.rs"]
pub mod external;

fn main() {}
