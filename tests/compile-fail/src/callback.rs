#[cfg(feature = "native")]
pub use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
pub use ffier_test_lib_via_cdylib as api;

pub struct BorrowedFruit<'a>(pub &'a i32);

impl api::Fruit for BorrowedFruit<'_> {
    fn value(&self) -> i32 {
        *self.0
    }
    fn try_count(&self, input: i32) -> Result<i32, api::TestError> {
        Ok(*self.0 + input)
    }
    fn count_tags(&self, tags: &[&str]) -> i32 {
        tags.len() as i32
    }
}
