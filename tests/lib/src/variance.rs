use std::cell::RefCell;

mod text {
    pub struct CovariantText<'a> {
        pub(super) value: &'a str,
    }
}

pub use text::CovariantText;

#[cfg_attr(feature = "ffi", ffier::export(send, sync, covariant('a)))]
impl<'a> text::CovariantText<'a> {
    pub fn new(value: &'a str) -> Self {
        Self { value }
    }

    pub fn value(&self) -> &str {
        self.value
    }
}

pub struct MixedVariance<'a, 'b, 'c, 'd> {
    read: &'a str,
    consume: fn(&'b str) -> usize,
    slot: RefCell<&'c str>,
    retained: &'d str,
}

#[cfg_attr(feature = "ffi", ffier::export(
    send, invariant('c), covariant('a, 'd), contravariant('b)
))]
impl<'a, 'b, 'c, 'd> MixedVariance<'a, 'b, 'c, 'd> {
    pub fn new(read: &'a str, _scope: &'b str, slot: &'c str, retained: &'d str) -> Self {
        Self {
            read,
            consume: str::len,
            slot: RefCell::new(slot),
            retained,
        }
    }

    pub fn read(&self) -> &str {
        self.read
    }

    pub fn consume(&self, value: &'b str) -> usize {
        (self.consume)(value)
    }

    pub fn replace(&self, value: &'c str) {
        *self.slot.borrow_mut() = value;
    }

    pub fn slot_len(&self) -> usize {
        self.slot.borrow().len()
    }

    pub fn retained(&self) -> &str {
        self.retained
    }
}

#[cfg(test)]
mod strict {
    #![deny(warnings)]

    // Cover leading underscores and a collision with the generated helper lifetime.
    pub struct NamedLifetimes<'_a, '__ffier_variance_long_0>(
        &'_a str,
        &'__ffier_variance_long_0 str,
    );

    #[cfg_attr(feature = "ffi", ffier::export(covariant('_a, '__ffier_variance_long_0)))]
    impl<'_a, '__ffier_variance_long_0> NamedLifetimes<'_a, '__ffier_variance_long_0> {
        pub fn new(first: &'_a str, second: &'__ffier_variance_long_0 str) -> Self {
            Self(first, second)
        }

        pub fn total_len(&self) -> usize {
            self.0.len() + self.1.len()
        }
    }

    #[test]
    fn lifetime_names_compile_without_warnings() {
        assert_eq!(NamedLifetimes::new("first", "second").total_len(), 11);
    }
}

#[cfg(test)]
mod tests {
    use crate as api;

    fn init() {}

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../variance_client.rs"
    ));
}
