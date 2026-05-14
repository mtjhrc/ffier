/// A simple trait defined in an external crate, with no ffier annotations.
pub trait Weighable {
    fn weight_grams(&self) -> i32;
}
