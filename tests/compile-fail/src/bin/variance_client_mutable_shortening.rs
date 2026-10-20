//! expected-error: lifetime may not live long enough
//! features: native via-cdylib weak-client

#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;
#[cfg(feature = "weak-client")]
mod api {
    #![allow(dead_code)]
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/variance-weak-client.rs"
    ));
}

// Covariance applies to owned values, not through &mut: a mutable reference
// makes the lifetime invariant.
fn shorten<'short, 'borrow>(
    value: &'borrow mut api::CovariantText<'static>,
) -> &'borrow mut api::CovariantText<'short> {
    value
}

fn main() {}
