/// A simple counter type, defined in a submodule.
pub struct Counter {
    value: i32,
}

#[ffier::exportable]
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

#[ffier::exportable]
impl Doubler {
    pub fn new() -> Self {
        Self
    }

    pub fn double(&self, counter: &Counter) -> i32 {
        counter.value * 2
    }
}
