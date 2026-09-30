#[path = "../callback.rs"]
mod callback;
use callback::{BorrowedFruit, api};

fn main() {
    let retained = api::RetainedFruit::new_static(api::Apple::new(1));
    let borrowed;
    {
        let local = 1;
        borrowed = BorrowedFruit(&local);
    }
    retained.borrowed_value(&borrowed);
}
