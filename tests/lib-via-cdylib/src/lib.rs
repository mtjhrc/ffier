// FFI client bindings — safe Rust wrappers calling through C ABI.
// These types mirror the original ffier-test-lib API but link dynamically
// via the cdylib (linked by build.rs).
//
// The bindings below are generated source code, NOT macro invocations.
// Regenerate with: just gen-rust-client
#![allow(clippy::all)]

include!("generated.rs");

// Manual extern for the hand-written debug bridge function.
unsafe extern "C" {
    pub fn ft_debug_fruit_dispatch_kind(handle: *mut core::ffi::c_void) -> ffier::FfierBytes;
}

#[cfg(test)]
mod tests {
    use super::*;

    // ================================================================
    // Constructors
    // ================================================================

    #[test]
    fn static_method_returning_self() {
        let w = Widget::new();
        assert_eq!(w.get_count(), 0);
    }

    #[test]
    fn static_method_returning_self_with_str_param() {
        let w = Widget::with_name("hello");
        assert_eq!(w.name(), "hello");
    }

    // ================================================================
    // Receiver patterns
    // ================================================================

    #[test]
    fn immutable_ref_method_returning_primitive() {
        let w = Widget::new();
        assert_eq!(w.get_count(), 0);
    }

    #[test]
    fn mutable_ref_method_void_return() {
        let mut w = Widget::new();
        w.set_count(42);
        assert_eq!(w.get_count(), 42);
    }

    #[test]
    fn by_value_method_void_return() {
        let w = Widget::new();
        w.consume();
    }

    // ================================================================
    // Primitive param/return types
    // ================================================================

    #[test]
    fn method_returning_bool() {
        let w = Widget::new();
        assert!(w.is_active());
    }

    #[test]
    fn method_with_i64_param_returning_i64() {
        let w = Widget::new();
        assert_eq!(w.negate(42), -42);
        assert_eq!(w.negate(-100), 100);
        assert_eq!(w.negate(0), 0);
    }

    // ================================================================
    // String/bytes returns
    // ================================================================

    #[test]
    fn method_returning_str() {
        let w = Widget::new();
        assert_eq!(w.name(), "widget");
    }

    #[test]
    fn method_returning_bytes() {
        let w = Widget::with_name("abc");
        assert_eq!(w.data(), b"abc");
    }

    #[test]
    fn method_with_str_param_returning_str() {
        let w = Widget::new();
        assert_eq!(w.echo("ping"), "ping");
    }

    // ================================================================
    // Str slice param
    // ================================================================

    #[test]
    fn method_with_str_slice_param() {
        let mut w = Widget::new();
        w.set_tags(&["alpha", "beta", "gamma"]);
        assert_eq!(w.tags_joined(), "alpha,beta,gamma");
    }

    // ================================================================
    // Result return patterns
    // ================================================================

    #[test]
    fn method_returning_result_void_ok() {
        let w = Widget::new();
        assert!(w.validate().is_ok());
    }

    #[test]
    fn method_returning_result_void_err() {
        let w = Widget::new();
        let err = w.fail_always().unwrap_err();
        assert_eq!(err, TestError::CustomMessage);
    }

    #[test]
    fn method_returning_result_value_ok() {
        let w = Widget::new();
        assert_eq!(w.parse_count("hello").unwrap(), 5);
    }

    #[test]
    fn method_returning_result_value_err() {
        let w = Widget::new();
        let err = w.parse_count("error").unwrap_err();
        assert_eq!(err, TestError::NotFound);
    }

    #[test]
    fn method_returning_result_str_ok() {
        let w = Widget::new();
        assert_eq!(w.describe(0).unwrap(), "zero");
        assert_eq!(w.describe(1).unwrap(), "one");
    }

    #[test]
    fn method_returning_result_str_err() {
        let w = Widget::new();
        let err = w.describe(99).unwrap_err();
        assert_eq!(err, TestError::NotFound);
    }

    #[test]
    fn method_returning_result_handle_ok() {
        let mut w = Widget::new();
        w.set_count(7);
        let g = w.try_create_gadget(true).unwrap();
        assert_eq!(g.get(), 7);
    }

    #[test]
    fn method_returning_result_handle_err() {
        let w = Widget::new();
        let err = w.try_create_gadget(false).unwrap_err();
        assert_eq!(err, TestError::NotFound);
    }

    #[test]
    fn method_returning_result_fail_with_value() {
        let w = Widget::new();
        let err = w.fail_with_value().unwrap_err();
        assert_eq!(err, TestError::InvalidInput);
    }

    // ================================================================
    // Handle as parameter
    // ================================================================

    #[test]
    fn method_with_borrowed_handle_param() {
        let mut w = Widget::new();
        w.set_count(10);
        let g = w.create_gadget();
        assert_eq!(w.read_gadget(&g), 10);
    }

    #[test]
    fn method_with_mutable_handle_param() {
        let w = Widget::new();
        let mut g = w.create_gadget();
        assert_eq!(g.get(), 0);
        w.update_gadget(&mut g, 99);
        assert_eq!(g.get(), 99);
    }

    #[test]
    fn method_returning_handle() {
        let mut w = Widget::new();
        w.set_count(33);
        let g = w.create_gadget();
        assert_eq!(g.get(), 33);
    }

    // ================================================================
    // Builder pattern (by-value self -> Self)
    // ================================================================

    #[test]
    fn builder_method_returning_self() {
        let c = Config::new().set_name("myconfig").set_size(42);
        assert_eq!(c.get_name(), "myconfig");
        assert_eq!(c.get_size(), 42);
    }

    #[test]
    fn builder_method_returning_result_self_ok() {
        let c = Config::new().set_name("valid").validated().unwrap();
        assert_eq!(c.get_name(), "valid");
    }

    #[test]
    fn builder_method_returning_result_self_err() {
        let err = Config::new().validated().unwrap_err();
        assert_eq!(err, TestError::InvalidInput);
    }

    #[test]
    fn builder_consuming_self_returning_other_handle() {
        let mut b = GizmoBuilder::new();
        b.set_name("mygizmo");
        b.set_size(100);
        let g = b.build();
        assert_eq!(g.name(), "mygizmo");
        assert_eq!(g.size(), 100);
    }

    #[test]
    fn builder_consuming_self_returning_result_handle_ok() {
        let mut b = GizmoBuilder::new();
        b.set_name("valid");
        b.set_size(50);
        let g = b.try_build().unwrap();
        assert_eq!(g.name(), "valid");
        assert_eq!(g.size(), 50);
    }

    #[test]
    fn builder_consuming_self_returning_result_handle_err() {
        let b = GizmoBuilder::new();
        // name empty — try_build() should fail
        let err = b.try_build().unwrap_err();
        assert_eq!(err, TestError::InvalidInput);
    }

    // ================================================================
    // Error type
    // ================================================================

    #[test]
    fn error_display() {
        assert_eq!(format!("{}", TestError::NotFound), "not found");
        assert_eq!(
            format!("{}", TestError::CustomMessage),
            "custom error message"
        );
        assert_eq!(format!("{}", TestError::InvalidInput), "invalid input");
    }

    // ================================================================
    // Lifetime-parameterized types
    // ================================================================

    #[test]
    fn lifetime_type_borrowing_handle() {
        let mut w = Widget::new();
        w.set_count(77);
        let v = View::create(&w);
        assert_eq!(v.source_count(), 77);
    }

    #[test]
    fn lifetime_type_str_methods() {
        let w = Widget::new();
        let mut v = View::create(&w);
        assert_eq!(v.label(), "default");
        v.set_label("custom");
        assert_eq!(v.label(), "custom");
    }

    // ================================================================
    // Lifetime-parameterized trait_impl
    // ================================================================

    #[test]
    fn lifetime_trait_impl_compiles() {
        let w = Widget::new();
        let v = View::create(&w);
        // Verifies impl<'a> Snapshot<'a> for View<'a> generated correctly
        let _: &dyn Snapshot = &v;
    }

    // ================================================================
    // Vtable / implementable
    // TODO: VtableProcessor needs `impl Processor` from the generator
    //       before these tests can work. See generated.rs.
    // ================================================================

    // use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    //
    // static LAST_NOTIFY_CODE: AtomicI32 = AtomicI32::new(-1);
    // static DROP_CALLED: AtomicBool = AtomicBool::new(false);
    //
    // unsafe extern "C" fn test_process(_self_data: *mut core::ffi::c_void, input: i32) -> i32 {
    //     input * 2
    // }
    //
    // unsafe extern "C" fn test_processor_name(
    //     _self_data: *mut core::ffi::c_void,
    // ) -> ffier::FfierBytes {
    //     ffier::FfierBytes::from_str("test_proc")
    // }
    //
    // unsafe extern "C" fn test_on_notify(_self_data: *mut core::ffi::c_void, code: i32) {
    //     LAST_NOTIFY_CODE.store(code, Ordering::SeqCst);
    // }
    //
    // unsafe extern "C" fn test_drop(_self_data: *mut core::ffi::c_void) {
    //     DROP_CALLED.store(true, Ordering::SeqCst);
    // }
    //
    // fn make_vtable() -> ProcessorVtable {
    //     ProcessorVtable {
    //         process: test_process,
    //         name: test_processor_name,
    //         on_notify: test_on_notify,
    //         drop: Some(test_drop),
    //     }
    // }
    //
    // #[test]
    // fn vtable_dyn_dispatch_process() {
    //     let mut p = Pipeline::new();
    //     LAST_NOTIFY_CODE.store(-1, Ordering::SeqCst);
    //     let vtable = make_vtable();
    //     let proc = VtableProcessor::new(std::ptr::null_mut(), &vtable);
    //     p.run(proc, 21);
    //     assert_eq!(LAST_NOTIFY_CODE.load(Ordering::SeqCst), 42);
    //     assert_eq!(p.result_count(), 1);
    //     assert_eq!(p.last_result().unwrap(), 42);
    // }
    //
    // #[test]
    // fn vtable_drop_callback() {
    //     DROP_CALLED.store(false, Ordering::SeqCst);
    //     let mut p = Pipeline::new();
    //     let vtable = make_vtable();
    //     let proc = VtableProcessor::new(std::ptr::null_mut(), &vtable);
    //     p.run(proc, 1);
    //     assert!(DROP_CALLED.load(Ordering::SeqCst));
    // }

    // ================================================================
    // Destroy (implicit via Drop)
    // ================================================================

    #[test]
    fn destroy_via_drop() {
        let _w = Widget::new();
        // Drop runs ft_widget_destroy automatically
    }
}
