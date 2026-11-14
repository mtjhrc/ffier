//! expected-error: PretendNever: ffier_rt::never_sealed::Sealed

pub struct PretendNever;

impl ffier::IsFfierNever for PretendNever {}

#[ffier::export]
#[ffier(never_type = PretendNever)]
pub fn run() -> PretendNever {
    PretendNever
}

fn main() {}
