//! expected-error: ffier exported method lifetime bounds and where clauses are unsupported

trait Backend {}
#[ffier::export]
pub fn bad<'a: 'b, 'b>(_backend: impl Backend + 'a) {}
fn main() {}
