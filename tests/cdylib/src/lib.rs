ffier_test_lib::__ffier_ft_library!(ffier_gen_c_macros::generate);

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
    use ffier::{FfiHandle, FfiType};
    let tag = unsafe { ffier::handle_type_tag(handle) };
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
    unsafe { ffier::FfierBytes::from_str(name) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    // ================================================================
    // Constructors
    // ================================================================

    #[test]
    fn static_method_returning_self() {
        unsafe {
            let w = ft_widget_new();
            assert!(!w.is_null());
            assert_eq!(ft_widget_get_count(w), 0);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn static_method_returning_self_with_str_param() {
        unsafe {
            let w = ft_widget_with_name(ffier::FfierBytes::from_str("hello"));
            assert!(!w.is_null());
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
            let r = ft_widget_validate(w, ptr::null_mut());
            assert_eq!(r, 0);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_void_err() {
        unsafe {
            let w = ft_widget_new();
            let r = ft_widget_fail_always(w, ptr::null_mut());
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 2); // CustomMessage
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_value_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("hello"),
                &mut result,
                ptr::null_mut(),
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
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("error"),
                &mut result,
                ptr::null_mut(),
            );
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 1); // NotFound
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_str_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut result = ffier::FfierBytes::EMPTY;
            let r = ft_widget_describe(w, 0, &mut result, ptr::null_mut());
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
            let r = ft_widget_describe(w, 99, &mut result, ptr::null_mut());
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 1); // NotFound
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_handle_ok() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 7);
            let mut g: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_try_create_gadget(w, true, &mut g, ptr::null_mut());
            assert_eq!(r, 0);
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
            let mut g: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_try_create_gadget(w, false, &mut g, ptr::null_mut());
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 1); // NotFound
            assert!(g.is_null());
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_fail_with_value() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let r = ft_widget_fail_with_value(w, &mut result, ptr::null_mut());
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 3); // InvalidInput
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
            assert!(!g.is_null());
            assert_eq!(ft_gadget_get(g), 33);
            ft_gadget_destroy(g);
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
            ft_config_set_name(&mut c, ffier::FfierBytes::from_str("myconfig"));
            ft_config_set_size(&mut c, 42);
            assert_eq!(ft_config_get_name(c).as_str_unchecked(), "myconfig");
            assert_eq!(ft_config_get_size(c), 42);
            ft_config_destroy(c);
        }
    }

    #[test]
    fn builder_method_returning_result_self_ok() {
        unsafe {
            let mut c = ft_config_new();
            ft_config_set_name(&mut c, ffier::FfierBytes::from_str("valid"));
            let r = ft_config_validated(&mut c, ptr::null_mut());
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
            let r = ft_config_validated(&mut c, ptr::null_mut());
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 3); // InvalidInput
                                                        // After error with by-value self, handle is consumed
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
            assert!(!g.is_null());
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
            let mut g: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_gizmo_builder_try_build(b, &mut g, ptr::null_mut());
            // b is consumed
            assert_eq!(r, 0);
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
            let mut g: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_gizmo_builder_try_build(b, &mut g, ptr::null_mut());
            // b is consumed
            assert_ne!(r, 0);
            assert_eq!(ffier::ffier_result_code(r), 3); // InvalidInput
            assert!(g.is_null());
        }
    }

    // ================================================================
    // Error type FFI
    // ================================================================

    #[test]
    fn error_code_constants() {
        use ffier::FfiError;
        let codes = ffier_test_lib::TestError::codes();
        assert!(codes
            .iter()
            .any(|&(name, val)| name == "NOT_FOUND" && val == 1));
        assert!(codes
            .iter()
            .any(|&(name, val)| name == "CUSTOM_MESSAGE" && val == 2));
        assert!(codes
            .iter()
            .any(|&(name, val)| name == "INVALID_INPUT" && val == 3));
    }

    #[test]
    fn strerror_auto_generated() {
        unsafe {
            let w = ft_widget_new();
            let r = ft_widget_fail_always(w, ptr::null_mut());
            assert_ne!(r, 0);
            let msg = CStr::from_ptr(ft_strerror(r)).to_str().unwrap();
            assert_eq!(msg, "custom error message");
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn strerror_not_found() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let r = ft_widget_parse_count(
                w,
                ffier::FfierBytes::from_str("error"),
                &mut result,
                ptr::null_mut(),
            );
            assert_ne!(r, 0);
            let msg = CStr::from_ptr(ft_strerror(r)).to_str().unwrap();
            assert_eq!(msg, "not found");
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn strerror_success() {
        unsafe {
            let msg = CStr::from_ptr(ft_strerror(0)).to_str().unwrap();
            assert_eq!(msg, "success");
        }
    }

    #[test]
    fn result_type_tag_and_code() {
        unsafe {
            let w = ft_widget_new();
            let r = ft_widget_fail_always(w, ptr::null_mut());
            // TestError has type_tag=1, CustomMessage has code=2
            assert_eq!(ffier::ffier_result_type_tag(r), 1);
            assert_eq!(ffier::ffier_result_code(r), 2);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_handle_message_and_destroy() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_fail_always(w, &mut err);
            assert_ne!(r, 0);
            assert!(!err.is_null());
            // ft_error_message returns the Display output
            let msg = ft_error_message(err);
            assert_eq!(msg.as_str_unchecked(), "custom error message");
            ft_error_destroy(err);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_handle_null_is_safe() {
        unsafe {
            // Passing NULL err_out is fine — no box is written
            let w = ft_widget_new();
            let r = ft_widget_fail_always(w, ptr::null_mut());
            assert_ne!(r, 0);
            ft_widget_destroy(w);

            // Destroying NULL is a no-op
            ft_error_destroy(ptr::null_mut());

            // Message on NULL returns empty
            let msg = ft_error_message(ptr::null());
            assert_eq!(msg.len, 0);
        }
    }

    #[test]
    fn error_handle_not_written_on_success() {
        unsafe {
            let w = ft_widget_new();
            let mut err: *mut core::ffi::c_void = ptr::null_mut();
            let r = ft_widget_validate(w, &mut err);
            assert_eq!(r, 0);
            // On success, err_out should still be NULL (not written)
            assert!(err.is_null());
            ft_widget_destroy(w);
        }
    }

    // ================================================================
    // Vtable / implementable
    // ================================================================

    static LAST_NOTIFY_CODE: AtomicI32 = AtomicI32::new(-1);
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

    unsafe extern "C" fn test_on_notify(_self_data: *mut core::ffi::c_void, code: i32) {
        LAST_NOTIFY_CODE.store(code, Ordering::SeqCst);
    }

    unsafe extern "C" fn test_drop(_self_data: *mut core::ffi::c_void) {
        DROP_CALLED.store(true, Ordering::SeqCst);
    }

    static PROCESSOR_VTABLE: ffier_test_lib::ProcessorVtable = ffier_test_lib::ProcessorVtable {
        drop: Some(test_drop),
        process: Some(test_process),
        name: Some(test_processor_name),
        on_notify: Some(test_on_notify),
    };

    fn make_processor_handle(user_data: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
        ft_processor_from_vtable(
            user_data,
            &PROCESSOR_VTABLE,
            core::mem::size_of_val(&PROCESSOR_VTABLE),
        )
    }

    #[test]
    fn vtable_dyn_dispatch_process() {
        unsafe {
            let p = ft_pipeline_new();
            LAST_NOTIFY_CODE.store(-1, Ordering::SeqCst);
            let proc = make_processor_handle(ptr::null_mut());
            ft_pipeline_run(p, proc, 21);
            assert_eq!(LAST_NOTIFY_CODE.load(Ordering::SeqCst), 42);
            assert_eq!(ft_pipeline_result_count(p), 1);
            let mut last: i32 = -1;
            let r = ft_pipeline_last_result(p, &mut last, ptr::null_mut());
            assert_eq!(r, 0);
            assert_eq!(last, 42);
            ft_pipeline_destroy(p);
        }
    }

    #[test]
    fn vtable_supertrait_method() {
        unsafe {
            let p = ft_pipeline_new();
            LAST_NOTIFY_CODE.store(-1, Ordering::SeqCst);
            let proc = make_processor_handle(ptr::null_mut());
            ft_pipeline_run(p, proc, 5);
            assert_eq!(LAST_NOTIFY_CODE.load(Ordering::SeqCst), 10);
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
        self_data as i32
    }

    unsafe extern "C" fn fruit_drop(_self_data: *mut core::ffi::c_void) {}

    // Static vtable variants for fruit tests — vtable must outlive handles.
    static FRUIT_VT_DROP_VALUE: ffier_test_lib::FruitVtable = ffier_test_lib::FruitVtable {
        drop: Some(fruit_drop),
        value: Some(fruit_value),
        label: None,
    };

    static FRUIT_VT_VALUE_ONLY: ffier_test_lib::FruitVtable = ffier_test_lib::FruitVtable {
        drop: None,
        value: Some(fruit_value),
        label: None,
    };

    #[test]
    fn mixer_blend_concrete() {
        unsafe {
            let m = ft_mixer_new();
            assert_eq!(
                ft_mixer_blend_concrete(m, ft_apple_new(10), ft_orange_new(20)),
                30
            );
            assert_eq!(ft_mixer_total(m), 30);
            ft_mixer_destroy(m);
        }
    }

    #[test]
    fn mixer_blend_hybrid() {
        unsafe {
            let m = ft_mixer_new();
            assert_eq!(
                ft_mixer_blend_hybrid(m, ft_apple_new(5), ft_banana_new(15)),
                20
            );
            assert_eq!(ft_mixer_total(m), 20);
            ft_mixer_destroy(m);
        }
    }

    #[test]
    fn mixer_blend_dynamic() {
        unsafe {
            let m = ft_mixer_new();
            assert_eq!(
                ft_mixer_blend_dynamic(m, ft_mango_new(3), ft_lemon_new(7)),
                10
            );
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
            // Create a VtableFruit via the C vtable mechanism.
            // fruit_value (defined above) reads self_data as i32.
            let handle = ft_fruit_from_vtable(
                77 as *mut core::ffi::c_void,
                &FRUIT_VT_DROP_VALUE,
                core::mem::size_of_val(&FRUIT_VT_DROP_VALUE),
            );
            assert_eq!(ft_fruit_value(handle), 77);
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
            let handle = ft_fruit_from_vtable(
                42 as *mut core::ffi::c_void,
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
    };

    #[test]
    fn vtable_default_method_overridden() {
        unsafe {
            // VtableFruit with label = Some(custom) → should use the custom impl
            let handle = ft_fruit_from_vtable(
                42 as *mut core::ffi::c_void,
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
            let truncated_size = core::mem::offset_of!(ffier_test_lib::FruitVtable, label);
            let handle = ft_fruit_from_vtable(
                42 as *mut core::ffi::c_void,
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
            let oversized = core::mem::size_of::<ffier_test_lib::FruitVtable>() + 64;
            let handle = ft_fruit_from_vtable(
                42 as *mut core::ffi::c_void,
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
            let size_for_drop_and_value = core::mem::offset_of!(ffier_test_lib::FruitVtable, label);
            let handle = ft_fruit_from_vtable(
                99 as *mut core::ffi::c_void,
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
            let handle = ft_fruit_from_vtable(
                42 as *mut core::ffi::c_void,
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
            let handle = ft_fruit_from_vtable(
                42 as *mut core::ffi::c_void,
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

            // Call ft_fruit_value — reads self_data as i32
            assert_eq!(ft_fruit_value(handle), 42);

            ft_fruit_destroy(handle);
        }
    }
}
