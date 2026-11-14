//! expected-error: `never_type` `u32` is not `ffier::Never`

type Never = ffier::Never;

#[ffier::export]
#[ffier(never_type = Never)]
pub fn run() -> u32 {
    42
}

fn main() {}
