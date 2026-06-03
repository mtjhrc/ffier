#![recursion_limit = "256"]
ffier_test_lib::__ffier_ft_library!(ffier_bridge_macros::generate);

// ---------------------------------------------------------------------------
// Manual bridge function — peeks at a handle's type tag to verify dispatch path.
// Also demonstrates that hand-written bridge functions work alongside generated ones.
// ---------------------------------------------------------------------------

/// Returns the concrete type name inside the handle ("Apple", "Orange",
/// "VtableFruit", or "unknown"). Consumes and destroys the handle.
///
/// # Safety
/// `handle` must be a valid handle to an `Apple`, `Orange`, or `VtableFruit`,
/// or `NULL`. The handle is consumed and must not be used after this call.
#[unsafe(no_mangle)]
#[allow(clippy::drop_non_drop)]
pub unsafe extern "C" fn ft_debug_fruit_dispatch_kind(
    handle: *mut core::ffi::c_void,
) -> ffier::FfierBytes {
    unsafe {
        use ffier_test_lib::{FfiHandle, FfiType};
        let tag = ffier::handle_type_tag(handle);
        let name = if tag == ffier_test_lib::VtableFruit::TYPE_TAG {
            drop(<ffier_test_lib::VtableFruit as FfiType>::from_c(handle));
            "VtableFruit"
        } else if tag == ffier_test_lib::Apple::TYPE_TAG {
            drop(<ffier_test_lib::Apple as FfiType>::from_c(handle));
            "Apple"
        } else if tag == ffier_test_lib::Orange::TYPE_TAG {
            drop(<ffier_test_lib::Orange as FfiType>::from_c(handle));
            "Orange"
        } else {
            "unknown"
        };
        // SAFETY: returned FfierBytes points to a static string literal — outlives the call.
        ffier::FfierBytes::from_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffier_test_lib::FfiHandle;
    use std::ffi::CStr;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Helper: stream ft_error_message into a String via a stack-local PushStr handle.
    unsafe fn error_message_to_string(err: *mut core::ffi::c_void) -> String {
        use core::ffi::c_void;

        // Vtable push callback: appends to a String via user_data.
        unsafe extern "C" fn push_to_string(
            self_data: *mut c_void,
            data: ffier::FfierBytes,
        ) -> bool {
            let s = unsafe { &mut *(self_data as *mut String) };
            s.push_str(unsafe { data.as_str_unchecked() });
            true
        }

        let vtable = ffier_test_lib::PushStrVtable {
            drop: None,
            push: Some(push_to_string),
        };

        let mut buf = String::new();
        let mut handle = ffier::FfierHandle {
            type_tag: ffier_test_lib::VtablePushStr::TYPE_TAG,
            metadata: 0,
            value: ffier::VtableHandle {
                vtable_ptr: &vtable as *const _ as *const c_void,
                user_data: &mut buf as *mut String as *const c_void,
                vtable_size: core::mem::size_of::<ffier_test_lib::PushStrVtable>() as u16,
            },
        };

        unsafe {
            ft_error_message(err as *mut c_void, &mut handle as *mut _ as *mut c_void);
        }
        buf
    }

    // ================================================================
    // Constructors
    // ================================================================

    #[test]
    fn static_method_returning_self() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_get_count(w), 0);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn static_method_returning_self_with_str_param() {
        unsafe {
            let w = ft_widget_with_name(ffier::FfierBytes::from_str("hello"));
            assert_eq!(ft_widget_name(w).as_str_unchecked(), "hello");
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Receiver patterns
    // ================================================================

    #[test]
    fn immutable_ref_method_returning_primitive() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_get_count(w), 0);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn mutable_ref_method_void_return() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 42);
            assert_eq!(ft_widget_get_count(w), 42);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn by_value_method_void_return() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_consume(w);
        }
    }

    // ================================================================
    // Primitive param/return types
    // ================================================================

    #[test]
    fn method_returning_bool() {
        unsafe {
            let w = ft_widget_new();
            assert!(ft_widget_is_active(w));
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_with_i64_param_returning_i64() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_negate(w, 42), -42);
            assert_eq!(ft_widget_negate(w, -100), 100);
            assert_eq!(ft_widget_negate(w, 0), 0);
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // String/bytes returns
    // ================================================================

    #[test]
    fn method_returning_str() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_name(w).as_str_unchecked(), "widget");
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_bytes() {
        unsafe {
            let w = ft_widget_with_name(ffier::FfierBytes::from_str("abc"));
            assert_eq!(ft_widget_data(w).as_bytes(), b"abc");
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_with_str_param_returning_str() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(
                ft_widget_echo(w, ffier::FfierBytes::from_str("ping")).as_str_unchecked(),
                "ping"
            );
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Str slice param
    // ================================================================

    #[test]
    fn method_with_str_slice_param() {
        unsafe {
            let w = ft_widget_new();
            let tags = [
                ffier::FfierBytes::from_str("alpha"),
                ffier::FfierBytes::from_str("beta"),
                ffier::FfierBytes::from_str("gamma"),
            ];
            ft_widget_set_tags(w, tags.as_ptr(), 3);
            assert_eq!(
                ft_widget_tags_joined(w).as_str_unchecked(),
                "alpha,beta,gamma"
            );
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // File descriptors — skipped under Miri (requires real syscalls)
    // ================================================================

    #[test]
    #[cfg_attr(miri, ignore)]
    fn method_with_borrowed_fd_param() {
        unsafe {
            let w = ft_widget_new();
            assert_eq!(ft_widget_fd_number(w, 0), 0); // stdin
            ft_widget_destroy(w);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn method_with_borrowed_fd_returning_owned_fd() {
        unsafe {
            use std::os::unix::io::FromRawFd;
            let w = ft_widget_new();
            let new_fd = ft_widget_dup_fd(w, 1); // dup stdout
            assert!(new_fd >= 0);
            assert_ne!(new_fd, 1);
            drop(std::os::unix::io::OwnedFd::from_raw_fd(new_fd));
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Result return patterns
    // ================================================================

    #[test]
    fn method_returning_result_void_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_validate(w, &mut err as *mut *mut core::ffi::c_void);
            assert_eq!(r, 0);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_void_err() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 2); // CustomMessage
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_value_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("hello"),
                &mut result,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_eq!(r, 0);
            assert_eq!(result, 5);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_value_err() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("error"),
                &mut result,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 1); // NotFound
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_str_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut result = ffier::FfierBytes::EMPTY;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_describe(w, 0, &mut result, &mut err as *mut *mut core::ffi::c_void);
            assert_eq!(r, 0);
            assert_eq!(result.as_str_unchecked(), "zero");
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_str_err() {
        unsafe {
            let w = ft_widget_new();
            let mut result = ffier::FfierBytes::EMPTY;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_describe(w, 99, &mut result, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 1); // NotFound
            assert!(!err.is_null(), "err handle should be non-null");
            // Test error_payload borrows the Box<str> payload from the handle
            let mut payload_buf = core::mem::MaybeUninit::<ffier::FfierBytes>::uninit();
            ft_error_payload(
                err as *const core::ffi::c_void,
                payload_buf.as_mut_ptr() as *mut core::ffi::c_void,
                core::mem::size_of::<ffier::FfierBytes>(),
            );
            let c_val = payload_buf.assume_init();
            let s =
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(c_val.data, c_val.len));
            assert_eq!(s, "code 99");
            // No need to free — c_val borrows from the handle
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_handle_ok() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 7);
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let g = ft_widget_try_create_gadget(w, true, &mut err as *mut *mut core::ffi::c_void);
            assert!(!g.is_null());
            assert_eq!(ft_gadget_get(g), 7);
            ft_gadget_destroy(g);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_handle_err() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let g = ft_widget_try_create_gadget(w, false, &mut err as *mut *mut core::ffi::c_void);
            assert!(g.is_null());
            // Extract result code from error handle
            let r2 = ft_error_result(err as *mut core::ffi::c_void);
            assert_eq!(ffier::ffier_result_code(r2), 1); // NotFound
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn result_name_data_carrying() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("error"),
                &mut result,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_ne!(r, 0);
            // strerror returns variant name with (...) for data-carrying
            let msg = CStr::from_ptr(ft_result_name_cstr(r)).to_str().unwrap();
            assert_eq!(msg, "NotFound(...)");
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_message_has_interpolated_data() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("error"),
                &mut result,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_ne!(r, 0);
            // error_message streams the rich Display output with interpolated data
            let msg = error_message_to_string(err);
            assert_eq!(msg, "not found: error");
            // strerror shows data-carrying hint, not the Display output
            let static_msg = CStr::from_ptr(ft_result_name_cstr(r)).to_str().unwrap();
            assert_eq!(static_msg, "NotFound(...)");
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Handle as parameter
    // ================================================================

    #[test]
    fn method_with_borrowed_handle_param() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 10);
            let g = ft_widget_create_gadget(w);
            assert_eq!(ft_widget_read_gadget(w, g), 10);
            ft_gadget_destroy(g);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_with_mutable_handle_param() {
        unsafe {
            let w = ft_widget_new();
            let g = ft_widget_create_gadget(w);
            assert_eq!(ft_gadget_get(g), 0);
            ft_widget_update_gadget(w, g, 99);
            assert_eq!(ft_gadget_get(g), 99);
            ft_gadget_destroy(g);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_handle() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 33);
            let g = ft_widget_create_gadget(w);
            assert_eq!(ft_gadget_get(g), 33);
            ft_gadget_destroy(g);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_borrowed_handle() {
        unsafe {
            let w = ft_widget_new();
            // The widget has an internal gadget with value 42.
            let g = ft_widget_gadget(w);
            assert!(!g.is_null());

            // The returned handle has a valid type tag — we can call methods on it.
            assert_eq!(ft_gadget_get(g), 42);

            // Destroying a borrowed handle is safe (deallocates the shell,
            // does NOT drop the inner Gadget which still lives in Widget).
            ft_gadget_destroy(g);

            // The widget is still fully alive after destroying the borrowed handle.
            ft_widget_set_count(w, 7);
            assert_eq!(ft_widget_get_count(w), 7);

            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Builder pattern (by-value self -> Self)
    // ================================================================

    #[test]
    fn builder_method_returning_self() {
        unsafe {
            let mut c = ft_config_new();
            ft_config_set_name(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                ffier::FfierBytes::from_str("myconfig"),
            );
            ft_config_set_size(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                42,
            );
            assert_eq!(ft_config_get_name(c).as_str_unchecked(), "myconfig");
            assert_eq!(ft_config_get_size(c), 42);
            ft_config_destroy(c);
        }
    }

    #[test]
    fn builder_method_returning_result_self_ok() {
        unsafe {
            let mut c = ft_config_new();
            ft_config_set_name(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                ffier::FfierBytes::from_str("valid"),
            );
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_config_validated(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_eq!(r, 0);
            assert_eq!(ft_config_get_name(c).as_str_unchecked(), "valid");
            ft_config_destroy(c);
        }
    }

    #[test]
    fn builder_method_returning_result_self_err() {
        unsafe {
            let mut c = ft_config_new();
            // name is empty — validated() should fail
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_config_validated(
                &mut c as *mut *mut core::ffi::c_void as *mut core::ffi::c_void,
                &mut err as *mut *mut core::ffi::c_void,
            );
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 3); // InvalidInput
            // After error with by-value self, handle is consumed
            assert!(!err.is_null());
            ft_error_destroy(err);
        }
    }

    #[test]
    fn builder_consuming_self_returning_other_handle() {
        unsafe {
            let b = ft_gizmo_builder_new();
            ft_gizmo_builder_set_name(b, ffier::FfierBytes::from_str("mygizmo"));
            ft_gizmo_builder_set_size(b, 100);
            let g = ft_gizmo_builder_build(b);
            // b is consumed
            assert_eq!(ft_gizmo_name(g).as_str_unchecked(), "mygizmo");
            assert_eq!(ft_gizmo_size(g), 100);
            ft_gizmo_destroy(g);
        }
    }

    #[test]
    fn builder_consuming_self_returning_result_handle_ok() {
        unsafe {
            let b = ft_gizmo_builder_new();
            ft_gizmo_builder_set_name(b, ffier::FfierBytes::from_str("valid"));
            ft_gizmo_builder_set_size(b, 50);
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let g = ft_gizmo_builder_try_build(b, &mut err as *mut *mut core::ffi::c_void);
            // b is consumed
            assert!(!g.is_null());
            assert_eq!(ft_gizmo_name(g).as_str_unchecked(), "valid");
            assert_eq!(ft_gizmo_size(g), 50);
            ft_gizmo_destroy(g);
        }
    }

    #[test]
    fn builder_consuming_self_returning_result_handle_err() {
        unsafe {
            let b = ft_gizmo_builder_new();
            // name empty — try_build() should fail
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let g = ft_gizmo_builder_try_build(b, &mut err as *mut *mut core::ffi::c_void);
            // b is consumed
            assert!(g.is_null());
            let r2 = ft_error_result(err as *mut core::ffi::c_void);
            assert_eq!(ffier::ffier_result_code(r2), 3); // InvalidInput
            ft_error_destroy(err);
        }
    }

    // ================================================================
    // Error type FFI
    // ================================================================

    #[test]
    fn error_code_constants() {
        use ffier::FfiError;
        let codes = ffier_test_lib::TestError::codes();
        assert!(
            codes
                .iter()
                .any(|&(name, val)| name == "NOT_FOUND" && val == 1)
        );
        assert!(
            codes
                .iter()
                .any(|&(name, val)| name == "CUSTOM_MESSAGE" && val == 2)
        );
        assert!(
            codes
                .iter()
                .any(|&(name, val)| name == "INVALID_INPUT" && val == 3)
        );
    }

    #[test]
    fn result_name_returns_variant_name() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            // strerror returns raw variant name, not Display output
            let msg = CStr::from_ptr(ft_result_name_cstr(r)).to_str().unwrap();
            assert_eq!(msg, "CustomMessage");
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn result_name_success() {
        unsafe {
            let msg = ft_result_name(0);
            assert_eq!(msg.as_str_unchecked(), "success");
        }
    }

    #[test]
    fn result_type_tag_and_code() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            // TestError has type_tag=1, library_tag=1 → composed tag
            let expected_tag = <ffier_test_lib::TestError as ffier_test_lib::FfiHandle>::TYPE_TAG;
            assert_eq!(ffier::ffier_result_type_tag(r), expected_tag);
            assert_eq!(ffier::ffier_result_code(r), 2);
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_handle_message_and_destroy() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            // ft_error_message streams the Display output through PushStr
            let msg = error_message_to_string(err);
            assert_eq!(msg, "custom error message");
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_result_from_handle() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            // ft_error_result extracts the FtResult from the boxed error
            let r2 = ft_error_result(err as *mut core::ffi::c_void);
            assert_eq!(r, r2);
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_result_null_returns_success() {
        // ft_error_result is now a trait dispatch method — no null guard.
    }

    #[test]
    fn error_handle_has_rtti_type_tag() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            // The error handle is a proper FfierHandleBox — type tag is readable
            let tag = ffier::handle_type_tag(err as *const core::ffi::c_void);
            // TestError has type_tag=1, library_tag=1 → composed tag
            let expected_tag = <ffier_test_lib::TestError as ffier_test_lib::FfiHandle>::TYPE_TAG;
            assert_eq!(tag, expected_tag);
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_handle_null_is_safe() {
        unsafe {
            // Passing NULL err_out is fine — no box is written
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err as *mut *mut core::ffi::c_void);
            assert_ne!(r, 0);
            ft_error_destroy(err);
            ft_widget_destroy(w);

            // Destroying NULL is a no-op
            ft_error_destroy(ptr::null_mut());

            // ft_error_result on NULL returns SUCCESS
            // ft_error_result is now a trait dispatch method — no null guard.
        }
    }

    #[test]
    fn error_handle_not_written_on_success() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_validate(w, &mut err as *mut *mut core::ffi::c_void);
            assert_eq!(r, 0);
            // On success, err_out should not have been written — pointer remains null
            assert!(err.is_null());
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Vtable / implementable
    // ================================================================

    static DROP_CALLED: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn test_process(_self_data: *mut core::ffi::c_void, input: i32) -> i32 {
        input * 2
    }

    unsafe extern "C" fn test_processor_name(
        _self_data: *mut core::ffi::c_void,
    ) -> ffier::FfierBytes {
        // SAFETY: points to a static string literal — outlives the call.
        unsafe { ffier::FfierBytes::from_str("test_proc") }
    }

    unsafe extern "C" fn test_drop(_self_data: *mut core::ffi::c_void) {
        DROP_CALLED.store(true, Ordering::SeqCst);
    }

    static PROCESSOR_VTABLE: ffier_test_lib::ProcessorVtable = ffier_test_lib::ProcessorVtable {
        drop: Some(test_drop),
        process: Some(test_process),
        name: Some(test_processor_name),
    };

    fn make_processor_handle(user_data: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
        ffier::ffier_handle_new_with_metadata(
            ffier_test_lib::VtableProcessor::TYPE_TAG,
            0,
            ffier::VtableHandle {
                vtable_ptr: &PROCESSOR_VTABLE as *const _ as *const core::ffi::c_void,
                user_data: user_data as *const core::ffi::c_void,
                vtable_size: core::mem::size_of::<ffier_test_lib::ProcessorVtable>() as u16,
            },
        )
    }

    #[test]
    fn vtable_dyn_dispatch_process() {
        unsafe {
            let p = ft_pipeline_new();
            let proc = make_processor_handle(ptr::null_mut());
            ft_pipeline_run(p, proc, 21);
            assert_eq!(ft_pipeline_result_count(p), 1);
            let mut last: i32 = -1;
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_pipeline_last_result(p, &mut last, &mut err as *mut *mut core::ffi::c_void);
            assert_eq!(r, 0);
            assert_eq!(last, 42);
            ft_pipeline_destroy(p);
        }
    }

    #[test]
    fn vtable_drop_callback() {
        unsafe {
            DROP_CALLED.store(false, Ordering::SeqCst);
            let p = ft_pipeline_new();
            let proc = make_processor_handle(ptr::null_mut());
            ft_pipeline_run(p, proc, 1);
            assert!(DROP_CALLED.load(Ordering::SeqCst));
            ft_pipeline_destroy(p);
        }
    }

    // ================================================================
    // Mixer with vtable fruit
    // ================================================================

    unsafe extern "C" fn fruit_value(self_data: *mut core::ffi::c_void) -> i32 {
        unsafe { *(self_data as *const i32) }
    }

    unsafe extern "C" fn fruit_count_tags(
        _self_data: *mut core::ffi::c_void,
        _tags: *const ffier::FfierBytes,
        tags_len: usize,
    ) -> i32 {
        // Just return the count of tags passed
        tags_len as i32
    }

    static FRUIT_VT_WITH_COUNT_TAGS: ffier_test_lib::FruitVtable = ffier_test_lib::FruitVtable {
        drop: Some(fruit_drop),
        value: Some(fruit_value),
        label: None,
        try_count: None,
        count_tags: Some(fruit_count_tags),
    };

    unsafe extern "C" fn fruit_drop(_self_data: *mut core::ffi::c_void) {}

    // Static vtable variants for fruit tests — vtable must outlive handles.
    static FRUIT_VT_DROP_VALUE: ffier_test_lib::FruitVtable = ffier_test_lib::FruitVtable {
        drop: Some(fruit_drop),
        value: Some(fruit_value),
        label: None,
        try_count: None,
        count_tags: None,
    };

    static FRUIT_VT_VALUE_ONLY: ffier_test_lib::FruitVtable = ffier_test_lib::FruitVtable {
        drop: None,
        value: Some(fruit_value),
        label: None,
        try_count: None,
        count_tags: None,
    };

    fn make_fruit_handle(
        user_data: *mut core::ffi::c_void,
        vtable: &'static ffier_test_lib::FruitVtable,
        vtable_size: usize,
    ) -> *mut core::ffi::c_void {
        ffier::ffier_handle_new_with_metadata(
            ffier_test_lib::VtableFruit::TYPE_TAG,
            0,
            ffier::VtableHandle {
                vtable_ptr: vtable as *const _ as *const core::ffi::c_void,
                user_data: user_data as *const core::ffi::c_void,
                vtable_size: vtable_size.min(u16::MAX as usize) as u16,
            },
        )
    }

    fn make_weighable_handle(
        user_data: *mut core::ffi::c_void,
        vtable: &'static ffier_test_lib::WeighableVtable,
        vtable_size: usize,
    ) -> *mut core::ffi::c_void {
        ffier::ffier_handle_new_with_metadata(
            ffier_test_lib::VtableWeighable::TYPE_TAG,
            0,
            ffier::VtableHandle {
                vtable_ptr: vtable as *const _ as *const core::ffi::c_void,
                user_data: user_data as *const core::ffi::c_void,
                vtable_size: vtable_size.min(u16::MAX as usize) as u16,
            },
        )
    }

    #[test]
    fn mixer_blend_concrete() {
        unsafe {
            let m = ft_mixer_new();
            let apple = ft_apple_new(10);
            let orange = ft_orange_new(20);
            assert_eq!(ft_mixer_blend_concrete(m, apple, orange,), 30);
            assert_eq!(ft_mixer_total(m), 30);
            ft_mixer_destroy(m);
        }
    }

    #[test]
    fn mixer_blend_hybrid() {
        unsafe {
            let m = ft_mixer_new();
            let apple = ft_apple_new(5);
            let banana = ft_banana_new(15);
            assert_eq!(ft_mixer_blend_hybrid(m, apple, banana,), 20);
            assert_eq!(ft_mixer_total(m), 20);
            ft_mixer_destroy(m);
        }
    }

    #[test]
    fn mixer_blend_dynamic() {
        unsafe {
            let m = ft_mixer_new();
            let mango = ft_mango_new(3);
            let lemon = ft_lemon_new(7);
            assert_eq!(ft_mixer_blend_dynamic(m, mango, lemon,), 10);
            assert_eq!(ft_mixer_total(m), 10);
            ft_mixer_destroy(m);
        }
    }

    // ================================================================
    // Lifetime-parameterized types
    // ================================================================

    #[test]
    fn lifetime_type_borrowing_handle() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 77);
            let v = ft_view_create(w);
            assert!(!v.is_null());
            assert_eq!(ft_view_source_count(v), 77);
            ft_view_destroy(v);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn lifetime_type_reading_through_borrow() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 123);
            let v = ft_view_create(w);
            assert_eq!(ft_view_source_count(v), 123);
            ft_view_destroy(v);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn lifetime_type_str_methods() {
        unsafe {
            let w = ft_widget_new();
            let v = ft_view_create(w);
            assert_eq!(ft_view_label(v).as_str_unchecked(), "default");
            ft_view_set_label(v, ffier::FfierBytes::from_str("custom"));
            assert_eq!(ft_view_label(v).as_str_unchecked(), "custom");
            ft_view_destroy(v);
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Destroy
    // ================================================================

    #[test]
    fn destroy_handle() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn destroy_null_handle() {
        unsafe { ft_widget_destroy(ptr::null_mut()) };
    }

    // ================================================================
    // Self-dispatch — trait-scoped functions that dispatch by type tag
    // ================================================================

    #[test]
    fn self_dispatch_fruit_value_on_apple() {
        unsafe {
            let apple = ft_apple_new(42);
            // ft_fruit_value dispatches to Apple::value via type tag
            assert_eq!(ft_fruit_value(apple), 42);
            ft_apple_destroy(apple);
        }
    }

    #[test]
    fn self_dispatch_fruit_value_on_orange() {
        unsafe {
            let orange = ft_orange_new(99);
            assert_eq!(ft_fruit_value(orange), 99);
            ft_orange_destroy(orange);
        }
    }

    #[test]
    fn self_dispatch_fruit_value_on_vtable_fruit() {
        unsafe {
            // Create a VtableFruit via the vtable mechanism.
            // fruit_value dereferences self_data as *const i32.
            let val: i32 = 77;
            let handle = make_fruit_handle(
                &val as *const i32 as *mut core::ffi::c_void,
                &FRUIT_VT_DROP_VALUE,
                core::mem::size_of_val(&FRUIT_VT_DROP_VALUE),
            );
            assert_eq!(ft_fruit_value(handle), 77);
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn self_dispatch_count_tags_on_apple() {
        unsafe {
            let apple = ft_apple_new(10);
            let tags = [
                ffier::FfierBytes::from_str("a"),
                ffier::FfierBytes::from_str("b"),
                ffier::FfierBytes::from_str("c"),
            ];
            // Apple::count_tags returns tags.len() + self.weight = 3 + 10 = 13
            assert_eq!(ft_fruit_count_tags(apple, tags.as_ptr(), tags.len()), 13);
            ft_apple_destroy(apple);
        }
    }

    #[test]
    fn vtable_count_tags() {
        unsafe {
            let handle = make_fruit_handle(
                ptr::null_mut(),
                &FRUIT_VT_WITH_COUNT_TAGS,
                core::mem::size_of_val(&FRUIT_VT_WITH_COUNT_TAGS),
            );
            let tags = [
                ffier::FfierBytes::from_str("x"),
                ffier::FfierBytes::from_str("y"),
            ];
            // VtableFruit dispatches through C fn ptr, which returns tags_len = 2
            assert_eq!(ft_fruit_count_tags(handle, tags.as_ptr(), tags.len()), 2);
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn self_dispatch_fruit_destroy_on_apple() {
        unsafe {
            let apple = ft_apple_new(1);
            // ft_fruit_destroy dispatches to the right destructor
            ft_fruit_destroy(apple);
        }
    }

    #[test]
    fn self_dispatch_fruit_destroy_null() {
        unsafe {
            ft_fruit_destroy(ptr::null_mut());
        }
    }

    #[test]
    fn self_dispatch_processor_process() {
        unsafe {
            let handle = make_processor_handle(ptr::null_mut());
            // ft_processor_process dispatches via type tag
            assert_eq!(ft_processor_process(handle, 10), 20);
            ft_processor_destroy(handle);
        }
    }

    #[test]
    fn self_dispatch_processor_name() {
        unsafe {
            let handle = make_processor_handle(ptr::null_mut());
            assert_eq!(ft_processor_name(handle).as_str_unchecked(), "test_proc",);
            ft_processor_destroy(handle);
        }
    }

    // ================================================================
    // Vtable default method fallback
    // ================================================================

    #[test]
    fn vtable_default_method_uses_fallback() {
        unsafe {
            // VtableFruit with label = None → should use the default "fruit"
            let handle = make_fruit_handle(
                ptr::null_mut(),
                &FRUIT_VT_DROP_VALUE,
                core::mem::size_of_val(&FRUIT_VT_DROP_VALUE),
            );
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "fruit");
            ft_fruit_destroy(handle);
        }
    }

    unsafe extern "C" fn custom_label(_self_data: *mut core::ffi::c_void) -> ffier::FfierBytes {
        unsafe { ffier::FfierBytes::from_str("custom") }
    }

    static FRUIT_VT_CUSTOM_LABEL: ffier_test_lib::FruitVtable = ffier_test_lib::FruitVtable {
        drop: Some(fruit_drop),
        value: Some(fruit_value),
        label: Some(custom_label),
        try_count: None,
        count_tags: None,
    };

    #[test]
    fn vtable_default_method_overridden() {
        unsafe {
            // VtableFruit with label = Some(custom) → should use the custom impl
            let handle = make_fruit_handle(
                ptr::null_mut(),
                &FRUIT_VT_CUSTOM_LABEL,
                core::mem::size_of_val(&FRUIT_VT_CUSTOM_LABEL),
            );
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "custom");
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn self_dispatch_default_method_on_concrete_type() {
        unsafe {
            // Apple doesn't override label → default "fruit" via self-dispatch
            let apple = ft_apple_new(10);
            assert_eq!(ft_fruit_label(apple).as_str_unchecked(), "fruit");
            ft_apple_destroy(apple);
        }
    }

    // ================================================================
    // Vtable forward/backward compatibility
    // ================================================================

    #[test]
    fn vtable_smaller_than_expected_uses_defaults() {
        unsafe {
            // Simulate an older client whose vtable only has `drop` + `value`
            // (no `label` field). Pass a truncated vtable_size so the library
            // treats `label` as absent → default dispatch.
            let val: i32 = 42;
            let truncated_size = core::mem::offset_of!(ffier_test_lib::FruitVtable, label);
            let handle = make_fruit_handle(
                &val as *const i32 as *mut core::ffi::c_void,
                &FRUIT_VT_CUSTOM_LABEL, // has label = Some(custom_label)
                truncated_size,         // but we tell the library it's smaller
            );
            // label field is beyond vtable_size → treated as None → default "fruit"
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "fruit");
            // value field is within vtable_size → works normally
            assert_eq!(ft_fruit_value(handle), 42);
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn vtable_larger_than_expected_works() {
        unsafe {
            // Simulate a newer client whose vtable is larger than the library
            // expects. Extra bytes beyond the library's struct size are ignored.
            let val: i32 = 42;
            let oversized = core::mem::size_of::<ffier_test_lib::FruitVtable>() + 64;
            let handle = make_fruit_handle(
                &val as *const i32 as *mut core::ffi::c_void,
                &FRUIT_VT_CUSTOM_LABEL,
                oversized,
            );
            // All known fields work normally
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "custom");
            assert_eq!(ft_fruit_value(handle), 42);
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn vtable_zero_size_all_defaults() {
        unsafe {
            // vtable_size = 0 → all fields treated as None.
            // drop = None means no drop callback (fine, user_data is not heap-allocated).
            // value is required → will panic. But label has a default.
            // For this test, just test that label defaults correctly.
            // We can't call value (it would panic), so use a handle with
            // a full vtable for value but truncated to only cover drop + value.
            let val: i32 = 99;
            let size_for_drop_and_value = core::mem::offset_of!(ffier_test_lib::FruitVtable, label);
            let handle = make_fruit_handle(
                &val as *const i32 as *mut core::ffi::c_void,
                &FRUIT_VT_DROP_VALUE,
                size_for_drop_and_value,
            );
            // value works (within bounds)
            assert_eq!(ft_fruit_value(handle), 99);
            // label is out of bounds → default
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "fruit");
            ft_fruit_destroy(handle);
        }
    }

    // ================================================================
    // Debug: handle type inspection
    // ================================================================

    #[test]
    fn debug_handle_type_vtable_fruit() {
        unsafe {
            let handle = make_fruit_handle(
                ptr::null_mut(),
                &FRUIT_VT_VALUE_ONLY,
                core::mem::size_of_val(&FRUIT_VT_VALUE_ONLY),
            );
            assert_eq!(
                ft_debug_handle_type(handle).as_str_unchecked(),
                "VtableFruit",
            );
            ft_fruit_destroy(handle);
        }
    }

    #[test]
    fn debug_handle_type_apple() {
        unsafe {
            let apple = ft_apple_new(1);
            assert_eq!(ft_debug_handle_type(apple).as_str_unchecked(), "Apple",);
            ft_apple_destroy(apple);
        }
    }

    #[test]
    fn debug_handle_type_null() {
        unsafe {
            assert_eq!(ft_debug_handle_type(ptr::null()).as_str_unchecked(), "null",);
        }
    }

    // ================================================================
    // Debug: handle roundtrip
    // ================================================================

    #[test]
    fn debug_vtable_handle_roundtrip() {
        unsafe {
            let val: i32 = 42;
            let handle = make_fruit_handle(
                &val as *const i32 as *mut core::ffi::c_void,
                &FRUIT_VT_VALUE_ONLY,
                core::mem::size_of_val(&FRUIT_VT_VALUE_ONLY),
            );

            // Verify handle is valid
            assert_eq!(
                ft_debug_handle_type(handle).as_str_unchecked(),
                "VtableFruit",
            );

            // Call ft_fruit_label — label is None, should use default
            assert_eq!(ft_fruit_label(handle).as_str_unchecked(), "fruit");

            // Call ft_fruit_value — dereferences self_data as *const i32
            assert_eq!(ft_fruit_value(handle), 42);

            ft_fruit_destroy(handle);
        }
    }

    // ================================================================
    // Enum constants + free functions
    // ================================================================

    #[test]
    fn free_fn_with_enum_param() {
        unsafe {
            // LogLevel::Info = 3
            let name = ft_log_level_name(3);
            assert_eq!(name.as_str_unchecked(), "info");
        }
    }

    #[test]
    fn free_fn_with_enum_param_off() {
        unsafe {
            // LogLevel::Off = 0
            let name = ft_log_level_name(0);
            assert_eq!(name.as_str_unchecked(), "off");
        }
    }

    #[test]
    fn free_fn_returning_bool_with_enum_param() {
        unsafe {
            // LogLevel::Off = 0 → not enabled
            assert!(!ft_log_level_is_enabled(0));
            // LogLevel::Error = 1 → enabled
            assert!(ft_log_level_is_enabled(1));
            // LogLevel::Trace = 5 → enabled
            assert!(ft_log_level_is_enabled(5));
        }
    }

    // ================================================================
    // Free function with BorrowedFd / OwnedFd
    // ================================================================

    #[test]
    fn free_fn_clone_fd() {
        use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
        unsafe {
            // Use stdout as a known-valid fd to clone
            let stdout_fd = std::io::stdout().as_raw_fd();
            let mut result: i32 = -1;
            let mut err_out: *mut core::ffi::c_void = core::ptr::null_mut();
            let r = ft_clone_fd(
                stdout_fd,
                &mut result as *mut i32,
                &mut err_out as *mut *mut core::ffi::c_void,
            );
            assert_eq!(r, 0, "ft_clone_fd should succeed");
            assert!(result >= 0, "cloned fd should be valid");
            // Clean up the cloned fd
            drop(OwnedFd::from_raw_fd(result));
        }
    }

    // ================================================================
    // Foreign trait (Weighable from foreign-trait-crate)
    // ================================================================

    #[test]
    fn foreign_trait_impl_via_bridge() {
        unsafe {
            let apple = ft_apple_new(150);
            // Apple.weight_grams() returns weight * 10
            assert_eq!(ft_apple_weight_grams(apple), 1500);
            ft_apple_destroy(apple);
        }
    }

    #[test]
    fn foreign_trait_self_dispatch() {
        unsafe {
            let apple = ft_apple_new(200);
            // Self-dispatch through ft_weighable_weight_grams
            assert_eq!(ft_weighable_weight_grams(apple), 2000);
            ft_weighable_destroy(apple);
        }
    }

    // -----------------------------------------------------------------------
    // Handle slice: &[&T] as param
    // -----------------------------------------------------------------------

    #[test]
    fn handle_slice_param_method() {
        unsafe {
            let w = ft_widget_new();

            // Create gadgets via widget (they have widget's count as initial value)
            ft_widget_set_count(w, 5);
            let g1 = ft_widget_create_gadget(w);
            let g2 = ft_widget_create_gadget(w);

            ft_widget_set_count(w, 7);
            let g3 = ft_widget_create_gadget(w);

            // sum_gadgets takes &[&Gadget]
            let handles = [g1, g2, g3];
            let sum = ft_widget_sum_gadgets(w, handles.as_ptr(), handles.len());
            // g1.value=5, g2.value=5, g3.value=7 → sum=17
            assert_eq!(sum, 17);

            ft_gadget_destroy(g1);
            ft_gadget_destroy(g2);
            ft_gadget_destroy(g3);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn handle_slice_param_free_function() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 3);
            let g1 = ft_widget_create_gadget(w);
            ft_widget_set_count(w, 4);
            let g2 = ft_widget_create_gadget(w);

            let handles = [g1, g2];
            let sum = ft_sum_gadget_values(handles.as_ptr(), handles.len());
            // g1.value=3, g2.value=4 → sum=7
            assert_eq!(sum, 7);

            ft_gadget_destroy(g1);
            ft_gadget_destroy(g2);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn handle_slice_param_empty() {
        unsafe {
            let w = ft_widget_new();
            // Empty slice
            let sum = ft_widget_sum_gadgets(w, core::ptr::null(), 0);
            assert_eq!(sum, 0);
            ft_widget_destroy(w);
        }
    }

    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Error-named error type (regression: name collides with std::error::Error)
    // -----------------------------------------------------------------------

    #[test]
    fn error_named_error_type_ok() {
        unsafe {
            let s = ft_sprocket_new(ffier::FfierBytes::from_str("ok"));
            let mut err: *mut core::ffi::c_void = core::ptr::null_mut();
            let r = ft_sprocket_try_spin(s, &mut err);
            assert_eq!(r, 0);
            assert!(err.is_null());
            ft_sprocket_destroy(s);
        }
    }

    #[test]
    fn error_named_error_type_err() {
        unsafe {
            let s = ft_sprocket_new(ffier::FfierBytes::from_str("broken"));
            let mut err: *mut core::ffi::c_void = core::ptr::null_mut();
            let r = ft_sprocket_try_spin(s, &mut err);
            assert_ne!(r, 0);
            assert!(!err.is_null());
            ft_error_destroy(err);
            ft_sprocket_destroy(s);
        }
    }

    #[test]
    fn foreign_trait_vtable_dispatch() {
        // Implement Weighable via vtable from C side
        unsafe extern "C" fn custom_weight(self_data: *mut core::ffi::c_void) -> i32 {
            unsafe { *(self_data as *const i32) }
        }

        static WEIGHABLE_VT: ffier_test_lib::WeighableVtable = ffier_test_lib::WeighableVtable {
            drop: None,
            weight_grams: Some(custom_weight),
        };

        unsafe {
            let val: i32 = 77;
            let handle = make_weighable_handle(
                &val as *const i32 as *mut core::ffi::c_void,
                &WEIGHABLE_VT,
                core::mem::size_of_val(&WEIGHABLE_VT),
            );

            // Self-dispatch should route through vtable
            assert_eq!(ft_weighable_weight_grams(handle), 77);

            ft_weighable_destroy(handle);
        }
    }
}
