use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{
    ErrorForwarder, Gadget, TestError, TestErrorFatalData, TestErrorOpaqueUnitData, Widget,
    ft_error_result, roundtrip_callback_error, test_error_drop_count,
};

struct ForwardingError {
    error: RefCell<Option<TestError>>,
    drops: Rc<Cell<usize>>,
}

impl ForwardingError {
    fn take_error(&self) -> TestError {
        self.error
            .borrow_mut()
            .take()
            .expect("callback called once")
    }
}

impl ErrorForwarder for ForwardingError {
    fn forward_value(&self) -> Result<i32, TestError> {
        Err(self.take_error())
    }

    fn forward_handle(&self) -> Result<Gadget, TestError> {
        Err(self.take_error())
    }

    fn forward_unit(&self) -> Result<(), TestError> {
        Err(self.take_error())
    }
}

impl Drop for ForwardingError {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

fn native_errors() -> [TestError; 7] {
    let widget = Widget::new();
    [
        widget.fail_always().unwrap_err(),
        widget.fail_with_value().unwrap_err(),
        widget.fail_with_number(76).unwrap_err(),
        widget.parse_count("error").unwrap_err(),
        widget.fail_with_mixed().unwrap_err(),
        widget.fail_with_opaque(false).unwrap_err(),
        widget.fail_with_opaque(true).unwrap_err(),
    ]
}

#[test]
fn opaque_payloads_use_the_same_variant_data_wrapper() {
    assert_eq!(
        size_of::<TestErrorFatalData>(),
        size_of::<*mut core::ffi::c_void>()
    );
    assert_eq!(
        size_of::<TestErrorOpaqueUnitData>(),
        size_of::<*mut core::ffi::c_void>()
    );
    let _: fn(TestErrorFatalData) -> TestError = TestError::Fatal;
    let _: fn(TestErrorOpaqueUnitData) -> TestError = TestError::OpaqueUnit;
}

#[test]
fn callback_error_preserves_native_handle_and_result() {
    for method in 0..3 {
        for error in native_errors() {
            let native_handle = error.handle_ptr();
            let native_result = unsafe { ft_error_result(native_handle) };
            let message = error.to_string();
            let before = test_error_drop_count();
            let drops = Rc::new(Cell::new(0));
            let callback = ForwardingError {
                error: RefCell::new(Some(error)),
                drops: drops.clone(),
            };
            let vtable = ForwardingError::__ffier_vtable();
            let user_data = &callback as *const ForwardingError as *mut core::ffi::c_void;
            let mut returned_handle = core::ptr::null_mut();
            let result = unsafe {
                match method {
                    0 => {
                        let mut value = 12345;
                        let result = vtable.forward_value.unwrap()(
                            user_data,
                            &mut value,
                            &mut returned_handle,
                        );
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
            if let TestError::MixedError(data) = &returned_error {
                assert_eq!((data.field_0(), data.field_2()), (17, 900));
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
fn callback_error_roundtrip_reconstructs_native_error_and_destroys_once() {
    for method in 0..3 {
        for error in native_errors() {
            let native_result = unsafe { ft_error_result(error.handle_ptr()) };
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
                unsafe { ft_error_result(error.handle_ptr()) },
                native_result
            );
            assert_eq!(error.to_string(), message);
            if let TestError::NumericError(data) = &error {
                assert_eq!(data.field_0(), 76);
            }
            if let TestError::MixedError(data) = &error {
                assert_eq!((data.field_0(), data.field_2()), (17, 900));
            }
            drop(error);
            assert_eq!(test_error_drop_count(), before + 1);
        }
    }
}
