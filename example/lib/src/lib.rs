/// Error codes for calculator operations. Zero means success.
#[repr(i32)]
#[derive(Clone, Copy)]
pub enum CalcError {
    DivisionByZero = 1,
    Overflow = 2,
}

impl ffier::FfiType for CalcError {
    type CRepr = i32;
    const C_TYPE_NAME: &str = "int32_t";
    fn into_c(self) -> i32 {
        self as i32
    }
    fn from_c(v: i32) -> Self {
        match v {
            1 => CalcError::DivisionByZero,
            2 => CalcError::Overflow,
            _ => panic!("invalid CalcError value: {v}"),
        }
    }
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
}
