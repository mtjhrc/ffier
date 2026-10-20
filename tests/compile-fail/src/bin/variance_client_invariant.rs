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

// 'c is invariant (interior RefCell): even through a shared reference the
// slot lifetime cannot change.
fn change_slot<'short, 'borrow>(
    value: &'borrow api::MixedVariance<'static, 'static, 'static, 'static>,
) -> &'borrow api::MixedVariance<'static, 'static, 'short, 'static> {
    value
}

fn main() {}
