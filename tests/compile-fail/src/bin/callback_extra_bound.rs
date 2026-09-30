trait Backend {}
#[ffier::export]
pub fn bad(_backend: impl Backend + Send) {}
fn main() {}
