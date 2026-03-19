#[derive(Clone, Copy, ffier::FfiError)]
pub enum CalcError {
    #[ffier(code = 1)]
    DivisionByZero,
    #[ffier(code = 2, message = "integer overflow")]
    Overflow,
}


#[derive(Default)]
pub struct MyCalculator {
    precision: u8,
    label: String,
}

#[ffier::exportable(prefix = "ex")]
impl MyCalculator {
    pub fn add(&self, a: i32, b: i32) -> i32 {
        a + b
    }

    pub fn multiply(&self, a: i32, b: i32) -> i32 {
        a * b
    }

    pub fn negate(&self, value: i64) -> i64 {
        -value
    }

    pub fn is_positive(&self, value: i32) -> bool {
        value > 0
    }

    pub fn set_precision(&mut self, bits: u8) {
        self.precision = bits;
    }

    pub fn divide(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        if b == 0 {
            Err(CalcError::DivisionByZero)
        } else {
            Ok(a / b)
        }
    }

    pub fn checked_add(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        a.checked_add(b).ok_or(CalcError::Overflow)
    }

    pub fn validate(&self) -> Result<(), CalcError> {
        Ok(())
    }

    pub fn name(&self) -> &str {
        if self.label.is_empty() {
            "calculator"
        } else {
            &self.label
        }
    }

    pub fn echo<'a>(&self, msg: &'a str) -> &'a str {
        msg
    }

    pub fn data(&self) -> &[u8] {
        self.label.as_bytes()
    }

    pub fn describe(&self, code: i32) -> Result<&str, CalcError> {
        match code {
            0 => Ok("ok"),
            1 => Ok("addition"),
            2 => Ok("subtraction"),
            _ => Err(CalcError::Overflow),
        }
    }
}
