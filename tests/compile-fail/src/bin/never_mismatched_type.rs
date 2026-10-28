//! expected-error: never_type must match the return type or the Result success type

type Never = ();

#[ffier::export]
#[ffier(never_type = Never)]
pub fn run() -> u32 {
    42
}

fn main() {}
