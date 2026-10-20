//! expected-error: ffier lifetime variance annotations are supported only on inherent impls

struct Hidden<'a>(&'a str);

trait VarianceWrongItem {
    fn value(&self) -> usize;
}

#[ffier::export(covariant('a))]
impl VarianceWrongItem for Hidden<'_> {
    fn value(&self) -> usize {
        0
    }
}

fn main() {}
