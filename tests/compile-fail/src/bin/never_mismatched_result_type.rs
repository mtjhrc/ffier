//! expected-error: `never_type` `u32` is not `ffier::Never`

type Never = ffier::Never;

pub enum RunError {}

#[ffier::export]
#[ffier(never_type = Never)]
pub fn run() -> Result<u32, RunError> {
    Ok(42)
}

fn main() {}
