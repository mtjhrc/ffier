//! expected-error: lifetime may not live long enough
//! features: native via-cdylib weak-client

#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(any(feature = "via-cdylib", feature = "weak-client"))]
use ffier_test_lib_via_cdylib as api;

// Contravariance lengthens, never shortens: a consumer accepting 'static
// arguments cannot be viewed as one accepting shorter lifetimes.
fn wrong_direction<'short, 'borrow>(
    value: &'borrow api::MixedVariance<'static, 'static, 'static, 'static>,
) -> &'borrow api::MixedVariance<'static, 'short, 'static, 'static> {
    value
}

fn main() {}
