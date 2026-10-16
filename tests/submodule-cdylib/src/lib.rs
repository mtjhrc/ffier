//! cdylib crate that generates C bridge functions for the submodule test library.
//! This verifies that types defined in submodules and referenced via `crate::`
//! paths in `library_definition!` produce working bridge functions.

ffier::generate_bridge!(
    external = ffier_test_submodule_lib::__ffier_subtest_metadata,
    schema_output = "../../target/ffier-subtest.json"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_bridge_preserves_payload_field_indices() {
        use ffier_test_submodule_lib::{FfiType, SubError};

        let handle = SubError::Mixed(17, (), 900).into_c();
        let mut first = 0i32;
        let mut last = 0u64;
        unsafe {
            subtest_error_payload(handle, 0, (&mut first as *mut i32).cast(), size_of::<i32>());
            subtest_error_payload(handle, 2, (&mut last as *mut u64).cast(), size_of::<u64>());
            assert_eq!((first, last), (17, 900));
            let error = SubError::from_c(handle);
            assert!(matches!(error, SubError::Mixed(17, (), 900)));
        }
    }

    #[test]
    #[should_panic(expected = "SubError::Mixed field 1 is opaque")]
    fn external_error_payload_rejects_opaque_field() {
        use ffier_test_submodule_lib::SubError;

        let error = SubError::Mixed(17, (), 900);
        let mut output = 0u64;
        unsafe {
            ffier::FfiError::payload(
                &error,
                1,
                (&mut output as *mut u64).cast(),
                size_of::<u64>(),
            );
        }
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
