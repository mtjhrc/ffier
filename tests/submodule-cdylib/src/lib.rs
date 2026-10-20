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
    fn external_bridge_separates_result_names_and_messages() {
        use ffier_test_submodule_lib::{FfiHandle, SubError};

        for (code, name, message) in [
            (1, "Oops", "Oops"),
            (2, "BadThing", "something went wrong"),
            (3, "Mixed", "Mixed"),
        ] {
            let result = ffier::ffier_result(SubError::TYPE_TAG, code);
            unsafe {
                assert_eq!(subtest_result_name(result).as_str_unchecked(), name);
                assert_eq!(subtest_result_message(result).as_str_unchecked(), message);
            }
        }
    }

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
    fn native_identity_round_trips_through_the_external_bridge() {
        use ffier_test_submodule_lib::{Counter, FfiHandle, FfiType};

        // into_c/from_c use the definition-site native alias; the value
        // must survive a round trip through the external bridge crate.
        let counter = Counter::new();
        let raw = counter.into_c();
        unsafe { assert_eq!(ffier::handle_type_tag(raw), Counter::TYPE_TAG) };
        let _restored: Counter = unsafe { FfiType::from_c(raw) };

        let handle = unsafe { subtest_counter_new() };
        unsafe {
            subtest_counter_increment(handle);
            subtest_counter_increment(handle);
            subtest_counter_increment(handle);
            assert_eq!(subtest_counter_get(handle), 3);
            subtest_counter_destroy(handle);
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
