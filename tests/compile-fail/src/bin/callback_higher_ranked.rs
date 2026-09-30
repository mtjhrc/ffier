trait Backend<'a> {}
#[ffier::export]
pub fn bad(_backend: impl for<'a> Backend<'a>) {}
fn main() {}
