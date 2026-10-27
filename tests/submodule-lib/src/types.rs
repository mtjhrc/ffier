/// A simple counter type, defined in a submodule.
pub struct Counter {
    value: i32,
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

#[ffier::export]
impl Counter {
    pub fn new() -> Self {
        Self { value: 0 }
    }

    pub fn increment(&mut self) {
        self.value += 1;
    }

    pub fn get(&self) -> i32 {
        self.value
    }
}

/// A second type that takes a Counter parameter (tests cross-type aliases).
pub struct Doubler;

impl Default for Doubler {
    fn default() -> Self {
        Self::new()
    }
}

/// A fixed-layout value struct defined in a submodule (tests qualified
/// registration paths for value structs).
#[ffier::export]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Reading {
    pub id: u32,
    pub value: i32,
}

#[ffier::export]
impl Doubler {
    pub fn new() -> Self {
        Self
    }

    pub fn double(&self, counter: &Counter) -> i32 {
        counter.value * 2
    }

    pub fn double_reading(&self, reading: &Reading) -> i32 {
        reading.value * 2
    }

    pub fn make_reading(&self, id: u32, value: i32) -> Reading {
        Reading { id, value }
    }
}
