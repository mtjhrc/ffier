//! expected-error: cannot move out of `owner` because it is borrowed
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

// The handle borrows from the owner; the owner cannot be destroyed first.
fn early_destroy() {
    let owner = String::from("borrowed");
    let value = api::CovariantText::new(&owner);
    drop(owner);
    assert_eq!(value.value(), "borrowed");
}

fn main() {}
