#[path = "../callback.rs"]
mod callback;
use api::Fruit;
use callback::{BorrowedFruit, api};

struct Bad(BorrowedFruit<'static>);
impl Fruit for Bad {
    fn value(&self) -> i32 {
        self.0.value()
    }
    fn try_count(&self, input: i32) -> Result<i32, api::TestError> {
        self.0.try_count(input)
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        self.0.count_tags(tags)
    }
    fn __borrow_callback_handle_mut(
        &mut self,
    ) -> ffier::CallbackHandle<'_, api::__FfierCallbackTagFruit, ffier::BorrowedCallback<true>>
    {
        self.__borrow_callback_handle()
    }
}
fn main() {}
