#[derive(Debug)]
#[cfg_attr(feature = "ffi", derive(ffier::FfiError))]
#[non_exhaustive]
pub enum LogError {
    #[cfg_attr(feature = "ffi", ffier(code = 1))]
    Unavailable,
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("logging unavailable")
    }
}
impl std::error::Error for LogError {}

#[cfg_attr(feature = "ffi", ffier::export)]
pub mod logging {
    use super::LogError;
    #[cfg(unix)]
    use std::os::fd::BorrowedFd;
    #[cfg(windows)]
    use std::os::windows::io::BorrowedHandle;

    pub const MARKER: u32 = 42;
    pub struct Marker;

    fn target_is_present(present: bool) -> bool {
        present
    }

    /// libkrun's Unix logging target borrows an optional file descriptor.
    #[cfg(unix)]
    pub fn init_log(target: Option<BorrowedFd<'static>>) -> Result<bool, LogError> {
        Ok(target_is_present(target.is_some()))
    }

    /// libkrun's Windows logging target borrows an optional HANDLE.
    #[cfg(windows)]
    pub fn init_log(target: Option<BorrowedHandle<'static>>) -> Result<bool, LogError> {
        Ok(target_is_present(target.is_some()))
    }

    pub mod nested {
        pub fn init_log() -> u32 {
            7
        }
    }
}

pub mod unrelated {
    pub fn init_log() -> u32 {
        11
    }
}

#[cfg(feature = "ffi")]
ffier::library_definition!("grouped", library_tag = 1,
    LogError = 1,
    trait ffier::builtins::PushStr = 2,
    trait ffier::builtins::Error = 3,
    Error for LogError,
    fn logging::init_log,
);

#[cfg(feature = "ffi")]
ffier::generate_bridge!(
    local = __ffier_grouped_metadata,
    schema_output = "../../target/ffier-grouped-functions.json"
);

#[cfg(test)]
mod tests {
    #[test]
    fn module_members_keep_their_rust_behavior() {
        assert!(!super::logging::init_log(None).unwrap());
        assert_eq!(super::logging::MARKER, 42);
        let _marker = super::logging::Marker;
        assert_eq!(super::logging::nested::init_log(), 7);
        assert_eq!(super::unrelated::init_log(), 11);
    }
}
