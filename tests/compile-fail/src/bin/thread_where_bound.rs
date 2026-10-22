//! expected-error: ffier exported trait where clauses are unsupported

#[ffier::export(no_vtable)]
pub trait WhereBackend
where
    Self: Send,
{
}

fn main() {}
