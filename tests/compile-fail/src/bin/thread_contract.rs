#[cfg(feature = "thread_false_send")]
struct NotSend(std::rc::Rc<i32>);
#[cfg(feature = "thread_false_send")]
#[ffier::export(send)]
impl NotSend {}

#[cfg(feature = "thread_false_sync")]
struct NotSync(std::cell::Cell<i32>);
#[cfg(feature = "thread_false_sync")]
#[ffier::export(sync)]
impl NotSync {}

#[cfg(any(
    feature = "thread_constrained_lifetime",
    feature = "thread_specialized_lifetime"
))]
struct Borrowed<'a>(&'a i32);

#[cfg(feature = "thread_constrained_lifetime")]
#[ffier::export(send)]
impl<'a: 'static> Borrowed<'a> {}

#[cfg(feature = "thread_specialized_lifetime")]
#[ffier::export(send)]
impl Borrowed<'static> {}

fn main() {}
