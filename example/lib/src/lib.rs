pub trait Calculator {
    fn add(&self, a: i32, b: i32) -> i32;
    fn multiply(&self, a: i32, b: i32) -> i32;
    fn negate(&self, value: i64) -> i64;
    fn is_positive(&self, value: i32) -> bool;
    fn set_precision(&mut self, bits: u8);
}

#[derive(Default)]
pub struct MyCalculator {
    precision: u8,
}

#[ffier::reflect]
impl Calculator for MyCalculator {
    fn add(&self, a: i32, b: i32) -> i32 {
        a + b
    }

    fn multiply(&self, a: i32, b: i32) -> i32 {
        a * b
    }

    fn negate(&self, value: i64) -> i64 {
        -value
    }

    fn is_positive(&self, value: i32) -> bool {
        value > 0
    }

    fn set_precision(&mut self, bits: u8) {
        self.precision = bits;
    }
}
