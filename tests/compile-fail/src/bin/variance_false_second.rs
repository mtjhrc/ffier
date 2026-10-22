//! expected-error: lifetime may not live long enough

// Variance is checked per parameter. 'b appears contravariantly
// (fn(&'b str)), so claiming covariance for it must fail.
struct Mixed<'a, 'b>(&'a str, fn(&'b str));

#[ffier::export(covariant('a, 'b))]
impl<'a, 'b> Mixed<'a, 'b> {}

fn main() {}
