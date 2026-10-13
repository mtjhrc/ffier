/// A simple counter type, defined in a submodule.
#[derive(Debug)]
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

#[ffier::export]
impl Doubler {
    pub fn new() -> Self {
        Self
    }

    pub fn double(&self, counter: &Counter) -> i32 {
        counter.value * 2
    }
}

#[ffier::export(cfg = "feature = \"extra\"")]
#[repr(C)]
pub struct ConfigValue {
    pub base: u32,
}

#[ffier::export]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScalarValues {
    pub i8_value: i8,
    pub i16_value: i16,
    pub i32_value: i32,
    pub i64_value: i64,
    pub u8_value: u8,
    pub u16_value: u16,
    pub u32_value: u32,
    pub u64_value: u64,
    pub offset: isize,
    pub size: usize,
    pub enabled: bool,
    pub f32_value: f32,
    pub f64_value: f64,
}

#[ffier::export]
pub fn echo_scalars(values: ScalarValues) -> ScalarValues {
    values
}
