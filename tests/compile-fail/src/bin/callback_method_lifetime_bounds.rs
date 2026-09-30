trait Backend {}
#[ffier::export]
pub fn bad<'a: 'b, 'b>(_backend: impl Backend + 'a) {}
fn main() {}
