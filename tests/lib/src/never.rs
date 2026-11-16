use crate::TestError;

/// Means "never returns" at the FFI boundary through `never_type`. The name is
/// deliberately not `Never`: ffier goes by the annotation, not the name. The
/// type must be `ffier::Never`'s.
#[rustversion::since(1.100)]
pub type NoReturn = !;
#[rustversion::before(1.100)]
pub type NoReturn = ();

// Success where the contract forbids it, to check that the bridge aborts.
// Only `()` can express it; with `!` Rust rejects it at compile time.
#[rustversion::before(1.100)]
fn break_contract() -> NoReturn {}
#[rustversion::since(1.100)]
fn break_contract() -> NoReturn {
    unreachable!("`!` has no values")
}
#[rustversion::before(1.100)]
fn break_result_contract() -> Result<NoReturn, TestError> {
    Ok(())
}
#[rustversion::since(1.100)]
fn break_result_contract() -> Result<NoReturn, TestError> {
    unreachable!("`!` has no values")
}

/// Shaped like libkrun's `Vmm::run_and_exit`: consumes its owner and only
/// returns on error.
#[derive(Default)]
pub struct NeverRun;

#[cfg_attr(feature = "ffi", ffier::export)]
impl NeverRun {
    pub fn new() -> Self {
        Self
    }

    /// `succeed` breaks the contract (see `break_result_contract`).
    #[cfg_attr(feature = "ffi", ffier(never_type = NoReturn))]
    pub fn run(self, succeed: bool) -> Result<self::NoReturn, TestError> {
        if succeed {
            break_result_contract()
        } else {
            Err(TestError::NumericError(73))
        }
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
pub trait NeverCallback {
    #[cfg_attr(feature = "ffi", ffier(index = 0, never_type = NoReturn))]
    fn run(&self) -> Result<NoReturn, TestError>;
}

#[cfg_attr(feature = "ffi", ffier::export)]
#[cfg_attr(feature = "ffi", ffier(never_type = NoReturn))]
pub fn run_never_callback(
    #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] callback: impl NeverCallback,
) -> Result<NoReturn, TestError> {
    callback.run()
}

/// Breaks the contract by returning (see `break_contract`).
#[cfg_attr(feature = "ffi", ffier::export)]
#[cfg_attr(feature = "ffi", ffier(never_type = NoReturn))]
pub fn never_return() -> self::NoReturn {
    break_contract()
}

/// A literal `!` needs no annotation.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn never_exit(status: i32) -> ! {
    std::process::exit(status)
}
