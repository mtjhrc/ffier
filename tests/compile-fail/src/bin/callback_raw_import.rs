//! features: via-cdylib
//! expected-error: unsafe and requires unsafe block

#[path = "../callback.rs"]
mod callback;
use callback::{BorrowedFruit, api};

fn main() {
    let local = 1;
    let retained = api::RetainedFruit::new_dynamic(BorrowedFruit(&local));
    let raw = retained.__into_raw();
    let _forged: api::RetainedFruit<'static> = api::RetainedFruit::__from_raw(raw);
}
