//! expected-error: lifetime may not live long enough
//! features: receiver-escapes

use ffier_test_lib_via_cdylib::View;

// The schema cannot infer the hidden Rust type's variance. Every lifetime of
// an opaque client handle must remain invariant, including through shared refs.
fn shorten<'short, 'borrow>(view: &'borrow View<'static>) -> &'borrow View<'short> {
    view
}

fn main() {}
