use std::ffi::CStr;

#[derive(Clone, Copy)]
pub enum CalcError {
    DivisionByZero,
    Overflow,
}

impl ffier::FfiError for CalcError {
    fn code(&self) -> u64 {
        match self {
            CalcError::DivisionByZero => 1,
            CalcError::Overflow => 2,
        }
    }

    fn static_message(code: u64) -> &'static CStr {
        match code {
            1 => c"division by zero",
            2 => c"integer overflow",
            _ => c"unknown calculator error",
        }
    }

    fn codes() -> &'static [(&'static str, u64)] {
        &[("DIVISION_BY_ZERO", 1), ("OVERFLOW", 2)]
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
