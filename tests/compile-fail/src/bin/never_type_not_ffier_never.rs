//! expected-error: `never_type` `u8` is not `ffier::Never`

type Never = u8;

#[ffier::export]
#[ffier(never_type = Never)]
pub fn run() -> Never {
    0
}

fn main() {}
