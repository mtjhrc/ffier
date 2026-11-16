//! `Self` returns from a reference receiver, which are new objects.

use crate::TestError;

/// A counter that can be copied.
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
