#[path = "../callback.rs"]
mod callback;
use callback::{BorrowedFruit, api};

fn main() {
    let retained;
    {
        let local = 1;
        retained = api::RetainedFruit::new_dynamic(BorrowedFruit(&local));
    }
    assert_eq!(retained.value(), 1);
}
