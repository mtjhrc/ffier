//! features: native via-cdylib
//! expected-error: does not live long enough

#[path = "../callback.rs"]
mod callback;
use callback::{BorrowedFruit, api};

fn main() {
    let local = 1;
    let retained = api::RetainedFruit::new_static(BorrowedFruit(&local));
    assert_eq!(retained.value(), 1);
}
