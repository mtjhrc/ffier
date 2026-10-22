//! features: via-cdylib
//! expected-error: does not live long enough

#[path = "../callback.rs"]
mod callback;
use api::Fruit;
use callback::{BorrowedFruit, api};

fn main() {
    let handle;
    {
        let local = 1;
        handle = BorrowedFruit(&local).__into_callback_handle();
    }
    drop(handle);
}
