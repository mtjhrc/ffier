ffier_test_lib::ffier_meta_op_widget!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_gadget!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_config!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_gizmo!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_gizmo_builder!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_view!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_pipeline!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_test_error!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_vtable_processor!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_apple!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_orange!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_vtable_fruit!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_fruit_for_apple!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_fruit_for_orange!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_mixer!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_sprocket!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_attachment_for_sprocket!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_view_factory!("ft", ffier_gen_c_macros::generate_bridge);

// ---------------------------------------------------------------------------
// Manual bridge function — peeks at a handle's TypeId to verify dispatch path.
// Also demonstrates that hand-written bridge functions work alongside generated ones.
// ---------------------------------------------------------------------------

/// Returns the concrete type name inside the handle ("Apple", "Orange",
/// "VtableFruit", or "unknown"). Consumes and destroys the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ft_debug_fruit_dispatch_kind(
    handle: *mut core::ffi::c_void,
) -> ffier::FfierBytes {
    use ffier::FfiType;
    let type_id = unsafe { ffier::handle_type_id(handle) };
    let name = if type_id == core::any::TypeId::of::<ffier_test_lib::VtableFruit>() {
        drop(<ffier_test_lib::VtableFruit as FfiType>::from_c(handle));
        "VtableFruit"
    } else if type_id == core::any::TypeId::of::<ffier_test_lib::Apple>() {
        drop(<ffier_test_lib::Apple as FfiType>::from_c(handle));
        "Apple"
    } else if type_id == core::any::TypeId::of::<ffier_test_lib::Orange>() {
        drop(<ffier_test_lib::Orange as FfiType>::from_c(handle));
        "Orange"
    } else {
        "unknown"
    };
    ffier::FfierBytes::from_str(name)
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
            let err = ft_widget_validate(w);
            assert_eq!(err.code, 0);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_void_err() {
        unsafe {
            let w = ft_widget_new();
            let mut err = ft_widget_fail_always(w);
            assert_eq!(err.code, 2); // CustomMessage
            err.free();
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_value_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let err = ft_widget_parse_count(w, ffier::FfierBytes::from_str("hello"), &mut result);
            assert_eq!(err.code, 0);
            assert_eq!(result, 5);
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_value_err() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err =
                ft_widget_parse_count(w, ffier::FfierBytes::from_str("error"), &mut result);
            assert_eq!(err.code, 1); // NotFound
            err.free();
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_str_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut result = ffier::FfierBytes::EMPTY;
            let err = ft_widget_describe(w, 0, &mut result);
            assert_eq!(err.code, 0);
            assert_eq!(result.as_str_unchecked(), "zero");
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_str_err() {
        unsafe {
            let w = ft_widget_new();
            let mut result = ffier::FfierBytes::EMPTY;
            let mut err = ft_widget_describe(w, 99, &mut result);
            assert_eq!(err.code, 1); // NotFound
            err.free();
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_handle_ok() {
        unsafe {
            let w = ft_widget_new();
            ft_widget_set_count(w, 7);
            let mut g: *mut core::ffi::c_void = ptr::null_mut();
            let err = ft_widget_try_create_gadget(w, true, &mut g);
            assert_eq!(err.code, 0);
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
            let mut err = ft_widget_try_create_gadget(w, false, &mut g);
            assert_eq!(err.code, 1); // NotFound
            assert!(g.is_null());
            err.free();
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn method_returning_result_fail_with_value() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err = ft_widget_fail_with_value(w, &mut result);
            assert_eq!(err.code, 3); // InvalidInput
            err.free();
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
            let err = ft_config_validated(&mut c);
            assert_eq!(err.code, 0);
            assert_eq!(ft_config_get_name(c).as_str_unchecked(), "valid");
            ft_config_destroy(c);
        }
    }

    #[test]
    fn builder_method_returning_result_self_err() {
        unsafe {
            let mut c = ft_config_new();
            // name is empty — validated() should fail
            let mut err = ft_config_validated(&mut c);
            assert_eq!(err.code, 3); // InvalidInput
            err.free();
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
            let err = ft_gizmo_builder_try_build(b, &mut g);
            // b is consumed
            assert_eq!(err.code, 0);
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
            let mut err = ft_gizmo_builder_try_build(b, &mut g);
            // b is consumed
            assert_eq!(err.code, 3); // InvalidInput
            assert!(g.is_null());
            err.free();
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
    fn error_message_auto_generated() {
        unsafe {
            let w = ft_widget_new();
            let mut err = ft_widget_fail_always(w);
            assert_eq!(err.code, 2);
            let msg = CStr::from_ptr(ft_test_error_message(&err))
                .to_str()
                .unwrap();
            assert_eq!(msg, "custom error message");
            err.free();
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_message_not_found() {
        unsafe {
            let w = ft_widget_new();
            let mut result: i32 = -1;
            let mut err =
                ft_widget_parse_count(w, ffier::FfierBytes::from_str("error"), &mut result);
            assert_eq!(err.code, 1);
            let msg = CStr::from_ptr(ft_test_error_message(&err))
                .to_str()
                .unwrap();
            assert_eq!(msg, "not found");
            err.free();
            ft_widget_destroy(w);
        }
    }

    #[test]
    fn error_free_resets_to_ok() {
        unsafe {
            let w = ft_widget_new();
            let mut err = ft_widget_fail_always(w);
            assert_ne!(err.code, 0);
            err.free();
            assert_eq!(err.code, 0);
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
        ffier::FfierBytes::from_str("test_proc")
    }

    unsafe extern "C" fn test_on_notify(_self_data: *mut core::ffi::c_void, code: i32) {
        LAST_NOTIFY_CODE.store(code, Ordering::SeqCst);
    }

    unsafe extern "C" fn test_drop(_self_data: *mut core::ffi::c_void) {
        DROP_CALLED.store(true, Ordering::SeqCst);
    }

    fn make_vtable() -> ffier_test_lib::ProcessorVtable {
        ffier_test_lib::ProcessorVtable {
            process: test_process,
            name: test_processor_name,
            on_notify: test_on_notify,
            drop: Some(test_drop),
        }
    }

    #[test]
    fn vtable_dyn_dispatch_process() {
        unsafe {
            let p = ft_pipeline_new();
            LAST_NOTIFY_CODE.store(-1, Ordering::SeqCst);
            let vtable = make_vtable();
            let proc = ft_processor_from_vtable(ptr::null_mut(), &vtable);
            ft_pipeline_run(p, proc, 21);
            assert_eq!(LAST_NOTIFY_CODE.load(Ordering::SeqCst), 42);
            assert_eq!(ft_pipeline_result_count(p), 1);
            let mut last: i32 = -1;
            let err = ft_pipeline_last_result(p, &mut last);
            assert_eq!(err.code, 0);
            assert_eq!(last, 42);
            ft_pipeline_destroy(p);
        }
    }

    #[test]
    fn vtable_supertrait_method() {
        unsafe {
            let p = ft_pipeline_new();
            LAST_NOTIFY_CODE.store(-1, Ordering::SeqCst);
            let vtable = make_vtable();
            let proc = ft_processor_from_vtable(ptr::null_mut(), &vtable);
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
            let vtable = make_vtable();
            let proc = ft_processor_from_vtable(ptr::null_mut(), &vtable);
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

    #[test]
    fn mixer_with_vtable_fruit() {
        unsafe {
            let mut m = ft_mixer_new();
            let vtable = ffier_test_lib::FruitVtable {
                value: fruit_value,
                drop: Some(fruit_drop),
            };
            let fruit = ft_fruit_from_vtable(7 as *mut core::ffi::c_void, &vtable);
            ft_mixer_add(&mut m, fruit);
            assert_eq!(ft_mixer_total(m), 7);
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
}
