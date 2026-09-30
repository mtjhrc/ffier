#[path = "../callback.rs"]
mod callback;
use api::Fruit;
use callback::{BorrowedFruit, api};

fn main() {
    let local = 1;
    let _raw = BorrowedFruit(&local).__into_raw_handle();
}
