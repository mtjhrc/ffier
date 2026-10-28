//! expected-error: #[ffier(never_type = ...)] requires an explicit return type

type Never = ();

#[ffier::export]
#[ffier(never_type = Never)]
pub fn run() {}

fn main() {}
