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

#[cfg(any(feature = "thread_stronger_bound", feature = "thread_shared_send"))]
#[ffier::export]
pub trait Backend {
    #[ffier(index = 0)]
    fn step(&self) -> i32;
}

#[cfg(feature = "thread_stronger_bound")]
#[ffier::export]
pub fn bad(#[ffier(dispatch = vtable)] _backend: impl Backend + Send) {}

#[cfg(feature = "thread_shared_send")]
#[ffier::export]
pub trait SendBackend: Send {
    #[ffier(index = 0)]
    fn step(&self) -> i32;
}
#[cfg(feature = "thread_shared_send")]
#[ffier::export]
pub fn bad(#[ffier(dispatch = vtable)] _backend: &impl SendBackend) {}

#[cfg(feature = "thread_stronger_bound")]
ffier::library_definition!("ui", library_tag = 1, trait Backend = 1, fn bad);
#[cfg(feature = "thread_shared_send")]
ffier::library_definition!("ui", library_tag = 1, trait SendBackend = 1, fn bad);

#[cfg(feature = "thread_supertrait")]
#[ffier::export]
pub trait Unsupported: Send + std::fmt::Debug {}

#[cfg(feature = "thread_where_bound")]
#[ffier::export(no_vtable)]
pub trait WhereBackend
where
    Self: Send,
{
}

#[cfg(any(feature = "thread_missing_annotation", feature = "thread_missing_definition"))]
pub struct Counter;
#[cfg(any(feature = "thread_missing_annotation", feature = "thread_missing_definition"))]
#[ffier::export]
impl Counter {}
#[cfg(any(feature = "thread_missing_annotation", feature = "thread_missing_definition"))]
#[ffier::export]
pub trait ThreadBackend: Send {
    #[ffier(index = 0)]
    fn step(&self) -> i32;
}
#[cfg(any(feature = "thread_missing_annotation", feature = "thread_missing_definition"))]
#[ffier::export]
impl ThreadBackend for Counter {
    fn step(&self) -> i32 {
        0
    }
}
#[cfg(feature = "thread_missing_annotation")]
ffier::library_definition!("ui", library_tag = 1,
    Counter = 1,
    trait ThreadBackend = 2,
    ThreadBackend for Counter,
);

#[cfg(any(
    feature = "thread_stronger_bound",
    feature = "thread_shared_send",
    feature = "thread_missing_annotation",
    feature = "thread_missing_definition"
))]
ffier::generate_bridge!(
    local = __ffier_ui_metadata,
    schema_output = "../../target/ui-thread-contract.json"
);

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

#[cfg(feature = "thread_missing_definition")]
#[ffier::export]
pub fn bad(_backend: impl ThreadBackend) {}

#[cfg(feature = "thread_missing_definition")]
ffier::library_definition!("ui", library_tag = 1,
    Counter = 1,
    ThreadBackend for Counter,
    fn bad,
);

fn main() {}
