//! expected-error: ffier opaque lifetime 'a requires an explicit variance annotation: covariant('a), contravariant('a), or invariant('a)

struct Missing<'a>(&'a str);

#[ffier::export]
impl<'a> Missing<'a> {}

fn main() {}
