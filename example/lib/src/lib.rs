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
    /// Add two integers.
    ///
    /// # Arguments
    ///
    /// * `a` - Left operand.
    /// * `b` - Right operand.
    ///
    /// # Returns
    ///
    /// The sum of `a` and `b`.
    pub fn add(&self, a: i32, b: i32) -> i32 {
        a + b
    }

    /// Multiply two integers.
    pub fn multiply(&self, a: i32, b: i32) -> i32 {
        a * b
    }

    /// Negate a 64-bit integer.
    pub fn negate(&self, value: i64) -> i64 {
        -value
    }

    /// Check whether a value is strictly positive.
    pub fn is_positive(&self, value: i32) -> bool {
        value > 0
    }

    /// Set the internal precision (number of bits).
    ///
    /// # Arguments
    ///
    /// * `bits` - Precision in bits.
    pub fn set_precision(&mut self, bits: u8) {
        self.precision = bits;
    }

    /// Divide `a` by `b`.
    ///
    /// # Arguments
    ///
    /// * `a` - Dividend.
    /// * `b` - Divisor (must not be zero).
    ///
    /// # Returns
    ///
    /// The quotient, or `DivisionByZero` if `b` is zero.
    pub fn divide(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        if b == 0 {
            Err(CalcError::DivisionByZero)
        } else {
            Ok(a / b)
        }
    }

    /// Add with overflow checking.
    pub fn checked_add(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        a.checked_add(b).ok_or(CalcError::Overflow)
    }

    /// Validate internal state.
    pub fn validate(&self) -> Result<(), CalcError> {
        Ok(())
    }

    /// Get the calculator's display name.
    ///
    /// # Returns
    ///
    /// A borrowed string referencing the internal label.
    pub fn name(&self) -> &str {
        if self.label.is_empty() {
            "calculator"
        } else {
            &self.label
        }
    }

    /// Echo back the given string (zero-copy).
    pub fn echo<'a>(&self, msg: &'a str) -> &'a str {
        msg
    }

    /// Get the raw label bytes.
    pub fn data(&self) -> &[u8] {
        self.label.as_bytes()
    }

    /// Look up a human-readable description for an operation code.
    ///
    /// # Arguments
    ///
    /// * `code` - Operation code (0=ok, 1=addition, 2=subtraction).
    ///
    /// # Returns
    ///
    /// The description string, or `Overflow` for unknown codes.
    pub fn describe(&self, code: i32) -> Result<&str, CalcError> {
        match code {
            0 => Ok("ok"),
            1 => Ok("addition"),
            2 => Ok("subtraction"),
            _ => Err(CalcError::Overflow),
        }
    }
}
