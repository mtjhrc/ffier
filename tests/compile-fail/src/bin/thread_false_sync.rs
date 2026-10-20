//! expected-error: cannot be shared between threads safely

struct NotSync(std::cell::Cell<i32>);

#[ffier::export(sync)]
impl NotSync {}

fn main() {}
