//! expected-error: lifetime may not live long enough

// Implied native bounds ('a: 'b) cannot justify the declared variance;
// the well-formedness check keeps both parameters independent.
struct Hidden<'a: 'b, 'b>(&'a &'b str);

#[ffier::export(covariant('a, 'b))]
impl<'a, 'b> Hidden<'a, 'b> {}

fn main() {}
