//! cdylib crate that generates C bridge functions for the submodule test library.
//! This verifies that types defined in submodules and referenced via `crate::`
//! paths in `library_definition!` produce working bridge functions.

ffier::generate_bridge!(
    external = ffier_test_submodule_lib::__ffier_subtest_metadata,
    schema_output = "../../target/ffier-subtest.json"
);

#[cfg(all(test, unix))]
mod fd_client_tests {
    #[allow(dead_code)]
    mod api {
        include!("generated.rs");
    }

    use std::os::fd::{AsFd, BorrowedFd, OwnedFd, RawFd};

    struct Backend {
        fd: OwnedFd,
        fail: bool,
    }

    impl api::FdSource for Backend {
        fn read_notify_fd(&self) -> Result<BorrowedFd<'_>, api::SubError> {
            if self.fail {
                Err(api::SubError::Oops)
            } else {
                Ok(self.fd.as_fd())
            }
        }

        fn clone_fd(&self, fd: BorrowedFd<'_>) -> Result<OwnedFd, api::SubError> {
            Ok(fd.try_clone_to_owned().unwrap())
        }

        fn preserve_fd(&self, fd: OwnedFd) -> OwnedFd {
            fd
        }

        fn raw_fd(&self, fd: RawFd) -> RawFd {
            fd
        }

        fn borrowed_fd<'a>(&self, fd: BorrowedFd<'a>) -> Option<BorrowedFd<'a>> {
            Some(fd)
        }
    }

    #[test]
    fn private_fd_types_work_through_cross_crate_callbacks() {
        use std::io::Read;
        let (stream, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let mut backend = Backend {
            fd: stream.into(),
            fail: false,
        };
        assert!(api::probe_fd_source(&mut backend).unwrap());
        backend.fail = true;
        assert_eq!(
            api::probe_fd_source(&mut backend).unwrap_err().to_string(),
            "oops"
        );
        drop(backend);
        // EOF also verifies that the callback's owned descriptor was closed.
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fieldless_error_factory_uses_defining_crate_constructor() {
        use ffier_test_submodule_lib::{FfiHandle, FfiType, SubError};

        for code in [1, 2] {
            let handle = subtest_error_new(ffier::ffier_result(SubError::TYPE_TAG, code));
            assert!(!handle.is_null());
            let error = unsafe { SubError::from_c(handle) };
            match code {
                1 => assert!(matches!(error, SubError::Oops)),
                2 => assert!(matches!(error, SubError::BadThing())),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn payload_factory_resolves_qualified_fields_in_external_bridge() {
        use ffier_test_submodule_lib::{types::Counter, FfiType, Inner, SubError};
        let message = String::from("external payload");
        let mut counter = Counter::new();
        counter.increment();
        let handle = unsafe {
            subtest_sub_error_payload_new(
                76,
                message.as_str().into_c(),
                counter.into_c(),
                Inner { value: 42 }.into_c(),
            )
        };
        drop(message);
        let error = unsafe { SubError::from_c(handle) };
        assert_eq!(error.to_string(), "76: external payload, 1, 42");
        assert_eq!(ffier::FfiError::code(&error), 3);
    }

    #[test]
    fn all_scalar_fields_cross_external_bridge() {
        use ffier_test_submodule_lib::{types::ScalarValues, FfiType};

        let values = ScalarValues {
            i8_value: i8::MIN,
            i16_value: i16::MIN,
            i32_value: i32::MIN,
            i64_value: i64::MIN,
            u8_value: u8::MAX,
            u16_value: u16::MAX,
            u32_value: u32::MAX,
            u64_value: u64::MAX,
            offset: isize::MIN,
            size: usize::MAX,
            enabled: true,
            f32_value: -0.0,
            f64_value: f64::INFINITY,
        };
        let result = unsafe { ScalarValues::from_c(subtest_echo_scalars(values.into_c())) };
        assert_eq!(result, values);
        assert_eq!(result.f32_value.to_bits(), values.f32_value.to_bits());
    }

    #[test]
    fn counter_new_and_get() {
        unsafe {
            let c = subtest_counter_new();
            assert!(!c.is_null());
            assert_eq!(subtest_counter_get(c), 0);
            subtest_counter_destroy(c);
        }
    }

    #[test]
    fn counter_increment() {
        unsafe {
            let c = subtest_counter_new();
            subtest_counter_increment(c);
            subtest_counter_increment(c);
            subtest_counter_increment(c);
            assert_eq!(subtest_counter_get(c), 3);
            subtest_counter_destroy(c);
        }
    }

    #[test]
    fn doubler_works() {
        unsafe {
            let c = subtest_counter_new();
            subtest_counter_increment(c);
            subtest_counter_increment(c);
            assert_eq!(subtest_counter_get(c), 2);

            let d = subtest_doubler_new();
            assert_eq!(subtest_doubler_double(d, c), 4);
            subtest_doubler_destroy(d);
            subtest_counter_destroy(c);
        }
    }

    #[test]
    fn destroy_null() {
        unsafe {
            subtest_counter_destroy(std::ptr::null_mut());
            subtest_doubler_destroy(std::ptr::null_mut());
        }
    }
}
