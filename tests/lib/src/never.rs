use crate::TestError;

// The annotation carries the FFI meaning; the alias name is deliberately not
// `Never`, and pure Rust builds continue to see an ordinary unit alias.
pub type NoReturn = ();

/// A VM-like operation consumes its owner and either diverges or returns an error.
#[derive(Default)]
pub struct NeverRun;

#[cfg_attr(feature = "ffi", ffier::export)]
impl NeverRun {
    pub fn new() -> Self {
        Self
    }

    #[cfg_attr(feature = "ffi", ffier(never_type = NoReturn))]
    pub fn run(self, return_success: bool) -> Result<NoReturn, TestError> {
        if return_success {
            // Deliberately violates the FFI contract for subprocess tests.
            Ok(())
        } else {
            Err(TestError::NumericError(73))
        }
    }
}

/// Fallible callbacks have no success value but can still report errors.
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait NeverCallback {
    #[cfg_attr(feature = "ffi", ffier(index = 0, never_type = NoReturn))]
    fn fail(&self, tags: &[&str]) -> Result<NoReturn, TestError>;

    #[cfg_attr(feature = "ffi", ffier(index = 1, never_type = NoReturn))]
    fn fail_default(&self) -> Result<NoReturn, TestError> {
        Err(TestError::InvalidInput())
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl NeverCallback for NeverRun {
    #[cfg_attr(feature = "ffi", ffier(never_type = NoReturn))]
    fn fail(&self, tags: &[&str]) -> Result<NoReturn, TestError> {
        assert_eq!(tags, &["guest"]);
        Err(TestError::CustomMessage)
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
#[cfg_attr(feature = "ffi", ffier(never_type = NoReturn))]
pub fn run_never_callback(
    #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] callback: impl NeverCallback,
    use_default: bool,
) -> Result<NoReturn, TestError> {
    if use_default {
        callback.fail_default()
    } else {
        callback.fail(&["guest"])
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
#[cfg_attr(feature = "ffi", ffier(never_type = NoReturn))]
pub fn never_error() -> Result<NoReturn, TestError> {
    Err(TestError::CustomMessage)
}

/// An invalid implementation used only in subprocess abort checks.
#[cfg_attr(feature = "ffi", ffier::export)]
#[cfg_attr(feature = "ffi", ffier(never_type = NoReturn))]
pub fn never_return() -> NoReturn {}

/// A real diverging function needs no additional annotation.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn never_exit(status: i32) -> ! {
    std::process::exit(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_error_only_operations_report_errors() {
        assert!(matches!(never_error(), Err(TestError::CustomMessage)));
        assert!(matches!(
            NeverRun::new().run(false),
            Err(TestError::NumericError(73))
        ));
        assert!(matches!(
            run_never_callback(NeverRun::new(), false),
            Err(TestError::CustomMessage)
        ));
        assert!(matches!(
            run_never_callback(NeverRun::new(), true),
            Err(TestError::InvalidInput())
        ));
    }

    #[cfg(not(feature = "ffi"))]
    #[test]
    fn ffi_off_keeps_an_ordinary_unit_alias() {
        let unit: () = never_return();
        assert_eq!(NeverRun::new().run(true).unwrap(), unit);
    }
}
