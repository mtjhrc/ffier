trait Backend {}
#[ffier::export]
pub fn bad(_backend: impl Backend + Unpin) {}
fn main() {}
