use crate::TestError;

pub type Never = ();

/// Shaped like libkrun's `Vmm::run_and_exit`: consumes its owner and only
/// returns on error.
#[derive(Default)]
pub struct NeverRun;

#[cfg_attr(feature = "ffi", ffier::export)]
impl NeverRun {
    pub fn new() -> Self {
        Self
    }

    pub fn run(self) -> Result<Never, TestError> {
        Err(TestError::NumericError(73))
    }

    pub fn exit(&self, status: i32) -> ! {
        std::process::exit(status)
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
pub trait NeverCallback {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn run(&self) -> Result<self::Never, TestError>;
}

#[cfg_attr(feature = "ffi", ffier::export)]
pub fn run_never_callback(
    #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] callback: impl NeverCallback,
) -> Result<Never, TestError> {
    callback.run()
}

#[cfg_attr(feature = "ffi", ffier::export)]
pub fn never_exit(status: i32) -> ! {
    std::process::exit(status)
}
