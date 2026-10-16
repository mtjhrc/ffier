use api::{Widget, ft_error_result, roundtrip_callback_error, test_error_drop_count};

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fieldless_error_client.rs"
));

fn payload_errors() -> [TestError; 2] {
    let widget = Widget::new();
    [
        widget.fail_with_number(76).unwrap_err(),
        widget.parse_count("error").unwrap_err(),
    ]
}

fn invoke_callback(
    callback: &ForwardingError,
    method: u32,
) -> (Option<ffier::FfierResult>, *mut core::ffi::c_void) {
    let vtable = ForwardingError::__ffier_vtable();
    let user_data = callback as *const ForwardingError as *mut core::ffi::c_void;
    let mut returned_handle = core::ptr::null_mut();
    let result = unsafe {
        match method {
            0 => {
                let mut value = 12345;
                let result =
                    vtable.forward_value.unwrap()(user_data, &mut value, &mut returned_handle);
                assert_eq!(value, 12345, "error callback wrote the success output");
                Some(result)
            }
            1 => {
                let value = vtable.forward_handle.unwrap()(user_data, &mut returned_handle);
                assert!(value.is_null());
                None
            }
            2 => Some(vtable.forward_unit.unwrap()(
                user_data,
                &mut returned_handle,
            )),
            _ => unreachable!(),
        }
    };
    (result, returned_handle)
}

#[test]
fn payload_callback_preserves_native_handle_and_result() {
    init();
    for method in 0..3 {
        for error in payload_errors() {
            let (native_handle, temporary) = error.message_handle();
            assert!(temporary.is_none());
            let native_result = unsafe { ft_error_result(native_handle) };
            let message = error.to_string();
            let before = test_error_drop_count();
            let drops = Rc::new(Cell::new(0));
            let callback = ForwardingError {
                error: RefCell::new(Some(error)),
                drops: drops.clone(),
            };
            let (result, returned_handle) = invoke_callback(&callback, method);

            // Check pointer identity before treating the output as a native handle.
            assert_eq!(
                returned_handle, native_handle,
                "callback replaced native handle"
            );
            let returned_error = unsafe { TestError::from_ffi(native_result, returned_handle) };
            if let Some(result) = result {
                assert_eq!(
                    result, native_result,
                    "callback changed type tag or variant code"
                );
            }
            assert_eq!(
                test_error_drop_count(),
                before,
                "callback destroyed transferred error"
            );
            assert_eq!(returned_error.to_string(), message);
            if let TestError::NumericError(data) = &returned_error {
                assert_eq!(data.field_0(), 76);
            }
            drop(callback);
            assert_eq!(drops.get(), 1);
            assert_eq!(test_error_drop_count(), before);
            drop(returned_error);
            assert_eq!(test_error_drop_count(), before + 1);
        }
    }
}

#[test]
fn payload_callback_roundtrip_reconstructs_native_error_and_destroys_once() {
    init();
    for method in 0..3 {
        for error in payload_errors() {
            let native_result = unsafe { ft_error_result(error.message_handle().0) };
            let message = error.to_string();
            let before = test_error_drop_count();
            let drops = Rc::new(Cell::new(0));
            let callback = ForwardingError {
                error: RefCell::new(Some(error)),
                drops: drops.clone(),
            };
            let error = roundtrip_callback_error(callback, method).unwrap_err();
            assert_eq!(drops.get(), 1);
            assert_eq!(test_error_drop_count(), before);
            assert_eq!(
                unsafe { ft_error_result(error.message_handle().0) },
                native_result
            );
            assert_eq!(error.to_string(), message);
            if let TestError::NumericError(data) = &error {
                assert_eq!(data.field_0(), 76);
            }
            drop(error);
            assert_eq!(test_error_drop_count(), before + 1);
        }
    }
}

#[test]
fn fieldless_callbacks_materialize_native_errors_and_preserve_results() {
    init();
    for method in 0..3 {
        for (error, code) in [
            (TestError::CustomMessage, 2),
            (TestError::InvalidInput(), 3),
        ] {
            let before = test_error_drop_count();
            let drops = Rc::new(Cell::new(0));
            let callback = ForwardingError {
                error: RefCell::new(Some(error)),
                drops: drops.clone(),
            };
            let (result, handle) = invoke_callback(&callback, method);
            assert!(!handle.is_null());
            let native_result = unsafe { ft_error_result(handle) };
            assert_eq!(ffier::ffier_result_code(native_result), code);
            assert_eq!(
                ffier::ffier_result_type_tag(native_result),
                ffier::compose_tag(1, 1)
            );
            if let Some(result) = result {
                assert_eq!(result, native_result);
            }
            assert_eq!(test_error_drop_count(), before);
            let error = unsafe { TestError::from_ffi(native_result, handle) };
            assert_eq!(test_error_drop_count(), before + 1);
            match code {
                2 => assert!(matches!(error, TestError::CustomMessage)),
                3 => assert!(matches!(error, TestError::InvalidInput())),
                _ => unreachable!(),
            }
            drop(error);
            drop(callback);
            assert_eq!(drops.get(), 1);
            assert_eq!(test_error_drop_count(), before + 1);
        }
    }
}

#[test]
fn fieldless_import_releases_handle_and_display_destroys_temporary() {
    init();
    let widget = Widget::new();
    let before = test_error_drop_count();
    let custom = widget.fail_always().unwrap_err();
    assert!(matches!(custom, TestError::CustomMessage));
    assert_eq!(test_error_drop_count(), before + 1);
    let invalid = widget.fail_with_value().unwrap_err();
    assert!(matches!(invalid, TestError::InvalidInput()));
    assert_eq!(test_error_drop_count(), before + 2);
    assert_eq!(custom.to_string(), "custom error message");
    assert_eq!(invalid.to_string(), "invalid input");
    assert_eq!(test_error_drop_count(), before + 4);
    drop(custom);
    drop(invalid);
    assert_eq!(test_error_drop_count(), before + 4);
}

#[test]
fn fieldless_roundtrip_destroys_one_native_error_per_callback() {
    init();
    for method in 0..3 {
        for error in [TestError::CustomMessage, TestError::InvalidInput()] {
            let before = test_error_drop_count();
            let drops = Rc::new(Cell::new(0));
            let error = roundtrip_callback_error(
                ForwardingError {
                    error: RefCell::new(Some(error)),
                    drops: drops.clone(),
                },
                method,
            )
            .unwrap_err();
            assert_eq!(drops.get(), 1);
            assert_eq!(test_error_drop_count(), before + 1);
            drop(error);
            assert_eq!(test_error_drop_count(), before + 1);
        }
    }
}
