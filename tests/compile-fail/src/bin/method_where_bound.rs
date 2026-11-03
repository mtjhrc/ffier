//! expected-error: ffier exported method lifetime bounds and where clauses are unsupported

#[ffier::export]
pub trait WhereBound {
    #[ffier(index = 0)]
    fn probe(&self) -> i32
    where
        Self: Clone;
}

fn main() {}
