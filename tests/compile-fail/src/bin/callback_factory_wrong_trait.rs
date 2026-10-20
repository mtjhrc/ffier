//! features: via-cdylib
//! expected-error: mismatched types

#[path = "../callback.rs"]
mod callback;
use api::Fruit;
use callback::api;

struct Bad;
impl Fruit for Bad {
    fn value(&self) -> i32 {
        1
    }
    fn try_count(&self, input: i32) -> Result<i32, api::TestError> {
        Ok(input)
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32
    }
    fn __into_callback_handle<'a>(self) -> ffier::CallbackHandle<'a, api::__FfierCallbackTagFruit>
    where
        Self: Sized + 'a,
    {
        api::Weighable::__into_callback_handle(api::Apple::new(1))
    }
}
fn main() {}
