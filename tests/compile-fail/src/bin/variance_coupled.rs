//! expected-error: lifetime may not live long enough

// Coupled constraints ('a: 'b, 'b: 'a) do not make the interior RefCell
// covariant; the declared variance must still be compiler-checked.
struct Coupled<'a: 'b, 'b: 'a>(std::cell::RefCell<&'a &'b str>);

#[ffier::export(covariant('a, 'b))]
impl<'a, 'b> Coupled<'a, 'b> {}

fn main() {}
