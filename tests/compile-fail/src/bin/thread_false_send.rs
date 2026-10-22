//! expected-error: cannot be sent between threads safely

struct NotSend(std::rc::Rc<i32>);

#[ffier::export(send)]
impl NotSend {}

fn main() {}
