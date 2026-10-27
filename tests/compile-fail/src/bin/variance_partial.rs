//! expected-error: ffier opaque lifetime 'b requires an explicit variance annotation: covariant('b), contravariant('b), or invariant('b)

struct Partial<'a, 'b>(&'a str, &'b str);

#[ffier::export(covariant('a))]
impl<'a, 'b> Partial<'a, 'b> {}

fn main() {}
