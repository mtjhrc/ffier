//! `#[ffier(returns_self)]` chaining, and `Self` returns that are new objects.

use crate::TestError;

/// A counter with chaining methods.
pub struct Counter {
    value: i32,
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl Counter {
    pub fn new(value: i32) -> Self {
        Counter { value }
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    /// Add to the value and return the receiver for chaining.
    #[cfg_attr(feature = "ffi", ffier(returns_self))]
    pub fn add(&mut self, n: i32) -> &mut Self {
        self.value += n;
        self
    }

    /// Add a non-negative amount, or fail without changing the value.
    #[cfg_attr(feature = "ffi", ffier(returns_self))]
    pub fn try_add(&mut self, n: i32) -> Result<&mut Self, TestError> {
        if n < 0 {
            return Err(TestError::InvalidInput());
        }
        self.value += n;
        Ok(self)
    }

    /// Return the receiver through a shared borrow.
    #[cfg_attr(feature = "ffi", ffier(returns_self))]
    pub fn peek(&self) -> &Self {
        self
    }

    /// A new counter with the same value: an ordinary owned return.
    pub fn duplicate(&self) -> Self {
        Counter::new(self.value)
    }

    /// Move the value into a new counter and reset this one.
    pub fn take(&mut self) -> Self {
        Counter::new(std::mem::take(&mut self.value))
    }

    /// A new counter with the same value, or an error when it is negative.
    pub fn try_duplicate(&self) -> Result<Self, TestError> {
        if self.value < 0 {
            return Err(TestError::InvalidInput());
        }
        Ok(self.duplicate())
    }

    /// Compiled out: an unmarked chaining method is not checked.
    #[cfg(any())]
    pub fn disabled_chain(&mut self) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use crate as api;

    fn init() {}

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../returns_self_client.rs"
    ));
}
