//! expected-error: ffier exported method lifetime bounds and where clauses are unsupported

#[ffier::export]
pub trait WhereStatic {
    #[ffier(index = 0)]
    fn probe<T>(&self) -> i32
    where
        T: 'static;
}

fn main() {}
