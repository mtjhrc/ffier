//! expected-error: cannot return value referencing local variable `local`
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

// CovariantText<'a> holds a borrow of 'a data: the value cannot outlive it.
fn escape<'a>() -> api::CovariantText<'a> {
    let local = String::from("local");
    api::CovariantText::new(&local)
}

fn main() {}
