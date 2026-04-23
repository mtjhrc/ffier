use std::io::Write;
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

// -- Error types --

#[derive(Clone, Copy, Debug, ffier::FfiError)]
pub enum CalcError {
    #[ffier(code = 1)]
    DivisionByZero,
}

#[derive(Clone, Copy, Debug, ffier::FfiError)]
pub enum BufferError {
    #[ffier(code = 1, message = "write failed")]
    WriteFailed,
}

// -- Calculator: primitives, results --

#[derive(Default)]
pub struct Calculator;

#[ffier::exportable]
impl Calculator {
    pub fn new() -> Self {
        Self
    }

    /// Add two integers.
    pub fn add(&self, a: i32, b: i32) -> i32 {
        a + b
    }

    /// Check whether a value is strictly positive.
    pub fn is_positive(&self, value: i32) -> bool {
        value > 0
    }

    /// Divide `a` by `b`, returning an error if `b` is zero.
    pub fn divide(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        if b == 0 {
            Err(CalcError::DivisionByZero)
        } else {
            Ok(a / b)
        }
    }
}

// -- TextBuffer: strings, bytes, file descriptors --

pub struct TextBuffer {
    contents: String,
    output_fd: OwnedFd,
}

#[ffier::exportable]
impl TextBuffer {
    /// Create a text buffer that writes to the given file descriptor.
    pub fn new(output_fd: OwnedFd) -> Self {
        Self {
            contents: String::new(),
            output_fd,
        }
    }

    /// Get the output file descriptor.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.output_fd.as_fd()
    }

    /// Append text to the buffer.
    pub fn write(&mut self, text: &str) {
        self.contents.push_str(text);
    }

    /// Append multiple strings to the buffer.
    pub fn write_parts(&mut self, parts: &[&str]) {
        for part in parts {
            self.contents.push_str(part);
        }
    }

    /// Get the buffer contents.
    pub fn contents(&self) -> &str {
        &self.contents
    }

    /// Get the buffer contents as raw bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.contents.as_bytes()
    }

    /// Flush the buffer contents to the output file descriptor.
    pub fn flush(&self) -> Result<(), BufferError> {
        let mut f = unsafe { std::fs::File::from_raw_fd(self.output_fd.as_raw_fd()) };
        let result = f.write_all(self.contents.as_bytes());
        std::mem::forget(f);
        result.map_err(|_| BufferError::WriteFailed)
    }

    pub fn clear(&mut self) {
        self.contents.clear();
    }
}

ffier::library_definition!(
    "mylib",
    Calculator = 1,
    CalcError = 2,
    TextBuffer = 3,
    BufferError = 4,
);
