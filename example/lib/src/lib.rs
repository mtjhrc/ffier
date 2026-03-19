#[derive(Clone, Copy, ffier::FfiError)]
pub enum CalcError {
    #[ffier(code = 1)]
    DivisionByZero,
    #[ffier(code = 2, message = "integer overflow")]
    Overflow,
}

pub trait Calculator {
    fn add(&self, a: i32, b: i32) -> i32;
    fn multiply(&self, a: i32, b: i32) -> i32;
    fn negate(&self, value: i64) -> i64;
    fn is_positive(&self, value: i32) -> bool;
    fn set_precision(&mut self, bits: u8);
    fn divide(&self, a: i32, b: i32) -> Result<i32, CalcError>;
    fn checked_add(&self, a: i32, b: i32) -> Result<i32, CalcError>;
    fn validate(&self) -> Result<(), CalcError>;
    fn name(&self) -> &str;
    fn echo<'a>(&self, msg: &'a str) -> &'a str;
    fn data(&self) -> &[u8];
    fn describe(&self, code: i32) -> Result<&str, CalcError>;
}

#[derive(Default)]
pub struct MyCalculator {
    precision: u8,
    label: String,
}

#[ffier::exportable(prefix = "ex")]
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

    fn divide(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        if b == 0 {
            Err(CalcError::DivisionByZero)
        } else {
            Ok(a / b)
        }
    }

    fn checked_add(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        a.checked_add(b).ok_or(CalcError::Overflow)
    }

    fn validate(&self) -> Result<(), CalcError> {
        Ok(())
    }

    fn name(&self) -> &str {
        if self.label.is_empty() {
            "calculator"
        } else {
            &self.label
        }
    }

    fn echo<'a>(&self, msg: &'a str) -> &'a str {
        msg
    }

    fn data(&self) -> &[u8] {
        self.label.as_bytes()
    }

    fn describe(&self, code: i32) -> Result<&str, CalcError> {
        match code {
            0 => Ok("ok"),
            1 => Ok("addition"),
            2 => Ok("subtraction"),
            _ => Err(CalcError::Overflow),
        }
    }
}
