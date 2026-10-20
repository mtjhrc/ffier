//! expected-error: ffier exported method lifetime bounds and where clauses are unsupported

#[ffier::export]
pub trait RawLifetime {
    #[ffier(index = 0, raw_handle)]
    fn retain<'a>(_data: &'a [u8])
    where
        'a: 'static,
        Self: Sized;
}

fn main() {}
