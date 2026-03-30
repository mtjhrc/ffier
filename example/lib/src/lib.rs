use std::os::unix::io::{AsRawFd, BorrowedFd};

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

#[ffier::exportable]
impl MyCalculator {
    /// Create a new calculator.
    pub fn new() -> Self {
        Self::default()
    }

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

    /// Accept a borrowed file descriptor (returns its raw fd number).
    pub fn fd_number(&self, fd: BorrowedFd<'_>) -> i32 {
        fd.as_raw_fd()
    }

    /// Set the label by joining strings.
    pub fn set_label(&mut self, parts: &[&str]) {
        self.label = parts.join("-");
    }

    /// Create a new result accumulator.
    pub fn create_result(&self) -> CalcResult {
        CalcResult { value: 0 }
    }

    /// Add a value into a result accumulator.
    pub fn accumulate(&self, res: &mut CalcResult, n: i32) {
        res.value += n;
    }

    /// Read the value from a result accumulator.
    pub fn read_result(&self, res: &CalcResult) -> i32 {
        res.value
    }

    /// Try to create a result with an initial divide.
    ///
    /// # Returns
    ///
    /// A new result accumulator initialized with the quotient.
    pub fn try_create_result(&self, a: i32, b: i32) -> Result<CalcResult, CalcError> {
        if b == 0 {
            Err(CalcError::DivisionByZero)
        } else {
            Ok(CalcResult { value: a / b })
        }
    }
}

#[derive(Default)]
pub struct CalcResult {
    value: i32,
}

#[ffier::exportable]
impl CalcResult {
    /// Get the accumulated value.
    pub fn get(&self) -> i32 {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffier::{FfiHandle, FfiType};

    #[test]
    fn type_ids_are_distinct() {
        let mc_id = <MyCalculator as FfiHandle>::type_id();
        let cr_id = <CalcResult as FfiHandle>::type_id();
        eprintln!("MyCalculator type_id = {mc_id:?}");
        eprintln!("CalcResult   type_id = {cr_id:?}");
        assert_ne!(mc_id, cr_id, "type_ids should be distinct");
    }

    #[test]
    fn handle_carries_type_id() {
        let handle = MyCalculator::default().into_c();
        let tid = unsafe { ffier::handle_type_id(handle) };
        eprintln!("handle type_id = {tid:?}");
        assert_eq!(tid, <MyCalculator as FfiHandle>::type_id());
        // cleanup
        let _ = MyCalculator::from_c(handle);
    }

    #[test]
    #[should_panic(expected = "is not a")]
    fn wrong_handle_type_panics() {
        // Create a CalcResult handle, then pretend it's a MyCalculator
        let wrong_handle = CalcResult::default().into_c();
        let actual = unsafe { ffier::handle_type_id(wrong_handle) };
        let expected = <MyCalculator as FfiHandle>::type_id();
        assert!(
            actual == expected,
            "ex_mycalculator_add(): `handle` is not a {} (expected type_id={:?}, got {:?})",
            <MyCalculator as FfiHandle>::C_HANDLE_NAME,
            expected,
            actual,
        );
    }
}
