//! expected-error: ffier callback lifetime bounds on generic parameters and where clauses are unsupported

trait Backend {}
#[ffier::export]
pub fn bad<'a: 'b, 'b>(_backend: impl Backend + 'a) {}
fn main() {}
