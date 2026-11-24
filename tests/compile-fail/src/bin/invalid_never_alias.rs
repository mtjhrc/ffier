//! expected-error: IsFfierNever

type Never = u32;

#[ffier::export]
pub fn run() -> Never {
    0
}

fn main() {}
