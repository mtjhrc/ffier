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

// Contravariance lengthens, never shortens: a consumer accepting 'static
// arguments cannot be viewed as one accepting shorter lifetimes.
fn wrong_direction<'short, 'borrow>(
    value: &'borrow api::ContravariantConsumer<'static>,
) -> &'borrow api::ContravariantConsumer<'short> {
    value
}

fn main() {}
