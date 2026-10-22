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

pub struct ContravariantConsumer<'a> {
    callback: fn(&'a str) -> usize,
}

#[cfg_attr(feature = "ffi", ffier::export(send, sync, contravariant('a)))]
impl<'a> ContravariantConsumer<'a> {
    pub fn new(_scope: &'a str) -> Self {
        Self { callback: str::len }
    }

    pub fn consume(&self, value: &'a str) -> usize {
        (self.callback)(value)
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

/// Lifetime names the variance machinery must tolerate. The inner
/// `#![deny(warnings)]` proves the generated variance checks stay
/// warning-free even for unusual parameter names.
#[cfg(test)]
mod strict {
    #![deny(warnings)]

    /// `'_a` is a valid lifetime name despite the leading underscore.
    pub struct UnderscoreLifetime<'_a> {
        value: &'_a str,
    }

    #[cfg_attr(feature = "ffi", ffier::export(covariant('_a)))]
    impl<'_a> UnderscoreLifetime<'_a> {
        pub fn new(value: &'_a str) -> Self {
            Self { value }
        }

        pub fn value(&self) -> &str {
            self.value
        }
    }

    /// Collides with the default lifetime name of the generated variance
    /// check helper; the macro must pick a fresh name for its long lifetime.
    pub struct HelperCollision<'__ffier_variance_long_0> {
        value: &'__ffier_variance_long_0 str,
    }

    #[cfg_attr(feature = "ffi", ffier::export(covariant('__ffier_variance_long_0)))]
    impl<'__ffier_variance_long_0> HelperCollision<'__ffier_variance_long_0> {
        pub fn new(value: &'__ffier_variance_long_0 str) -> Self {
            Self { value }
        }

        pub fn value(&self) -> &str {
            self.value
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[allow(clippy::needless_lifetimes)]
        fn shorten_underscore<'long: 'short, 'short>(
            value: UnderscoreLifetime<'long>,
        ) -> UnderscoreLifetime<'short> {
            value
        }

        #[allow(clippy::needless_lifetimes)]
        fn shorten_collision<'long: 'short, 'short>(
            value: HelperCollision<'long>,
        ) -> HelperCollision<'short> {
            value
        }

        #[test]
        fn underscore_prefixed_lifetime_name_is_valid() {
            let source = String::from("underscore");
            let value = shorten_underscore(UnderscoreLifetime::new(&source));
            assert_eq!(value.value(), source);
        }

        #[test]
        fn helper_colliding_lifetime_name_is_valid() {
            let source = String::from("collision");
            let value = shorten_collision(HelperCollision::new(&source));
            assert_eq!(value.value(), source);
        }
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
