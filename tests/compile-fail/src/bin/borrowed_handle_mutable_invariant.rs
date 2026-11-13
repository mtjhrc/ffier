//! expected-error: lifetime may not live long enough
//! features: via-cdylib weak-client

use ffier_test_lib_via_cdylib as api;

// The handle is covariant, but exclusive access must keep its type invariant.
fn shorten<'borrow, 'short: 'borrow>(
    value: api::RefMut<'borrow, api::CovariantText<'static>>,
) -> api::RefMut<'borrow, api::CovariantText<'short>> {
    value
}

fn main() {}
