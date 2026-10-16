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
