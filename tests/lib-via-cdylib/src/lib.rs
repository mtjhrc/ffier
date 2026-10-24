// FFI client bindings — safe Rust wrappers calling through C ABI.
// These types mirror the original ffier-test-lib API but link dynamically
// via the cdylib (linked by build.rs).
//
// The bindings below are generated source code, NOT macro invocations.
// build.rs regenerates them from the library's JSON schema on every build.
#![allow(clippy::should_implement_trait)]

include!(concat!(env!("OUT_DIR"), "/generated.rs"));
include!("../../load_dll.rs");

// Manual extern for the hand-written debug bridge function.
unsafe extern "C" {
    pub fn ft_debug_fruit_dispatch_kind(handle: *mut core::ffi::c_void) -> ffier::FfierBytes;
}

#[cfg(test)]
fn init() {
    #[cfg(all(windows, not(feature = "strong")))]
    {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| unsafe {
            set_library_handle(Some(load_test_dll("ffier_test_lib")), LoadPolicy::Explicit)
                .unwrap();
        });
    }
    require(include!(concat!(env!("OUT_DIR"), "/required_symbols.rs"))).unwrap();
    ffier_test_foreign_lib_via_cdylib::init_for_tests();
}

#[cfg(test)]
mod callback_errors {
    use super::init;
    use crate as api;
    include!("callback_errors.rs");
}

#[cfg(all(test, not(feature = "strong")))]
#[allow(dead_code)]
mod lazy_callback_errors {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));

    mod tests {
        use crate::lazy_callback_errors as api;

        fn init() {
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| {
                #[cfg(windows)]
                let handle = Some(crate::load_test_dll("ffier_test_lib"));
                #[cfg(not(windows))]
                let handle = None;
                unsafe { api::set_library_handle(handle, api::LoadPolicy::Lazy) }.unwrap()
            });
        }

        include!("callback_errors.rs");
    }
}

#[cfg(test)]
mod unloaded_fieldless_errors {
    #[cfg(feature = "strong")]
    use crate as api;

    #[cfg(not(feature = "strong"))]
    #[allow(dead_code)]
    mod api {
        include!(concat!(env!("OUT_DIR"), "/generated.rs"));
    }

    #[test]
    fn fieldless_values_need_no_symbols_and_display_only_needs_result_message() {
        #[cfg(all(windows, not(feature = "strong")))]
        unsafe {
            api::set_library_handle(
                Some(crate::load_test_dll("ffier_test_lib")),
                api::LoadPolicy::Explicit,
            )
            .unwrap();
        }
        let custom = api::TestError::CustomMessage;
        let invalid = api::TestError::InvalidInput();
        assert!(matches!(custom, api::TestError::CustomMessage));
        assert!(matches!(invalid, api::TestError::InvalidInput()));
        drop(custom);
        drop(invalid);

        use api::Symbol;
        api::require(&[Symbol::FtResultMessage, Symbol::FtTestErrorDropCount]).unwrap();
        let before = api::test_error_drop_count();
        assert_eq!(
            api::TestError::CustomMessage.to_string(),
            "custom error message"
        );
        assert_eq!(api::TestError::InvalidInput().to_string(), "invalid input");
        assert_eq!(api::test_error_drop_count(), before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_empty_trait_vtable_has_no_method_slots() {
        init();
        use std::cell::Cell;
        use std::mem::size_of;
        use std::rc::Rc;

        unsafe extern "C" {
            fn ft_empty_trait_destroy(handle: *mut core::ffi::c_void);
            fn ft_retired_trait_destroy(handle: *mut core::ffi::c_void);
        }

        struct Marker(Rc<Cell<usize>>);
        impl EmptyTrait for Marker {}
        impl RetiredTrait for Marker {}
        impl Drop for Marker {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let slot_size = size_of::<Option<unsafe extern "C" fn()>>();
        let header_size = size_of::<ffier::VtableHeader>();
        let empty = <Marker as EmptyTrait>::__ffier_vtable();
        assert_eq!(empty._num_slots, 0);
        assert_eq!(size_of::<EmptyTraitVtable>(), header_size + slot_size);
        let retired = <Marker as RetiredTrait>::__ffier_vtable();
        assert_eq!(retired._num_slots, 3);
        assert_eq!(size_of::<RetiredTraitVtable>(), header_size + 4 * slot_size);
        assert!(retired.__reserved_0.is_none());
        assert!(retired.__reserved_1.is_none());
        assert!(retired.__reserved_2.is_none());

        let drops = Rc::new(Cell::new(0));
        unsafe {
            ft_empty_trait_destroy(EmptyTrait::__into_raw_handle(Marker(drops.clone())));
        }
        assert_eq!(drops.get(), 1);
        unsafe {
            ft_retired_trait_destroy(RetiredTrait::__into_raw_handle(Marker(drops.clone())));
        }
        assert_eq!(drops.get(), 2);
    }

    #[test]
    fn multiple_callbacks_share_prepared_allocation_and_drop_once() {
        init();
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct ClientFruit(i32, Arc<AtomicUsize>);
        impl Fruit for ClientFruit {
            fn value(&self) -> i32 {
                self.0
            }
            fn try_count(&self, input: i32) -> Result<i32, TestError> {
                Ok(self.0 + input)
            }
            fn count_tags(&self, tags: &[&str]) -> i32 {
                tags.len() as i32
            }
        }
        impl Drop for ClientFruit {
            fn drop(&mut self) {
                self.1.fetch_add(1, Ordering::Relaxed);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let mut mixer = Mixer::new();
        assert_eq!(
            mixer.blend_dynamic(
                ClientFruit(19, drops.clone()),
                ClientFruit(23, drops.clone())
            ),
            42
        );
        assert_eq!(drops.load(Ordering::Relaxed), 2);
        assert_eq!(
            mixer.blend_dynamic(Apple::new(5), ClientFruit(4, drops.clone())),
            9
        );
        assert_eq!(drops.load(Ordering::Relaxed), 3);
    }

    #[cfg(any(not(feature = "strong"), feature = "optional-entry"))]
    #[test]
    fn optional_api_is_discoverable_without_its_schema_feature() {
        init();
        require(&[
            Symbol::FtOptionalWidgetNew,
            Symbol::FtOptionalWidgetDestroy,
            Symbol::FtOptionalWidgetBase,
            Symbol::FtOptionalWidgetAmplify,
            Symbol::FtOptionalApply,
            Symbol::FtOptionalModeName,
            Symbol::FtOptionalMergeFlags,
        ])
        .unwrap();
        assert!(Symbol::FtOptionalWidgetNew.is_available());
        let widget = OptionalWidget::new(7);
        assert_eq!(widget.base(), 7);
        assert_eq!(widget.amplify(6), 42);
        assert_eq!(optional_apply(widget, 6), 42);
        assert_eq!(optional_mode_name(OptionalMode::Advanced), "advanced");
        assert_eq!(
            optional_merge_flags(OptionalFlags::READ, OptionalFlags::WRITE).bits(),
            3
        );
    }

    // ================================================================
    // Constructors
    // ================================================================

    #[test]
    fn static_method_returning_self() {
        init();
        let w = Widget::new();
        assert_eq!(w.get_count(), 0);
    }

    #[test]
    fn static_method_returning_self_with_str_param() {
        init();
        let w = Widget::with_name("hello");
        assert_eq!(w.name(), "hello");
    }

    // ================================================================
    // Receiver patterns
    // ================================================================

    #[test]
    fn immutable_ref_method_returning_primitive() {
        init();
        let w = Widget::new();
        assert_eq!(w.get_count(), 0);
    }

    #[test]
    fn mutable_ref_method_void_return() {
        init();
        let mut w = Widget::new();
        w.set_count(42);
        assert_eq!(w.get_count(), 42);
    }

    #[test]
    fn by_value_method_void_return() {
        init();
        let w = Widget::new();
        w.consume();
    }

    // ================================================================
    // Primitive param/return types
    // ================================================================

    #[test]
    fn method_returning_bool() {
        init();
        let w = Widget::new();
        assert!(w.is_active());
    }

    #[test]
    fn method_with_i64_param_returning_i64() {
        init();
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
        init();
        let w = Widget::new();
        assert_eq!(w.name(), "widget");
    }

    #[test]
    fn method_returning_bytes() {
        init();
        let w = Widget::with_name("abc");
        assert_eq!(w.data(), b"abc");
    }

    #[test]
    fn method_with_str_param_returning_str() {
        init();
        let w = Widget::new();
        assert_eq!(w.echo("ping"), "ping");
    }

    // ================================================================
    // Str slice param
    // ================================================================

    #[test]
    fn method_with_str_slice_param() {
        init();
        let mut w = Widget::new();
        w.set_tags(&["alpha", "beta", "gamma"]);
        assert_eq!(w.tags_joined(), "alpha,beta,gamma");
    }

    // ================================================================
    // Result return patterns
    // ================================================================

    #[test]
    fn method_returning_result_void_ok() {
        init();
        let w = Widget::new();
        assert!(w.validate().is_ok());
    }

    #[test]
    fn method_returning_result_void_err() {
        init();
        let w = Widget::new();
        let err = w.fail_always().unwrap_err();
        assert!(matches!(err, TestError::CustomMessage));
    }

    #[test]
    fn method_returning_result_value_ok() {
        init();
        let w = Widget::new();
        assert_eq!(w.parse_count("hello").unwrap(), 5);
    }

    #[test]
    fn method_returning_result_value_err() {
        init();
        let w = Widget::new();
        let err = w.parse_count("error").unwrap_err();
        assert!(matches!(err, TestError::NotFound(..)));
    }

    #[test]
    fn method_returning_result_str_ok() {
        init();
        let w = Widget::new();
        assert_eq!(w.describe(0).unwrap(), "zero");
        assert_eq!(w.describe(1).unwrap(), "one");
    }

    #[test]
    fn method_returning_result_str_err() {
        init();
        let w = Widget::new();
        let err = w.describe(99).unwrap_err();
        assert!(matches!(err, TestError::NotFound(..)));
    }

    #[test]
    fn method_returning_result_handle_ok() {
        init();
        let mut w = Widget::new();
        w.set_count(7);
        let g = w.try_create_gadget(true).unwrap();
        assert_eq!(g.get(), 7);
    }

    #[test]
    fn method_returning_result_handle_err() {
        init();
        let w = Widget::new();
        let err = w.try_create_gadget(false).unwrap_err();
        assert!(matches!(err, TestError::NotFound(..)));
    }

    #[test]
    fn method_returning_result_fail_with_value() {
        init();
        let w = Widget::new();
        let err = w.fail_with_value().unwrap_err();
        assert!(matches!(err, TestError::InvalidInput(..)));
    }

    // ================================================================
    // Handle as parameter
    // ================================================================

    #[test]
    fn method_with_borrowed_handle_param() {
        init();
        let mut w = Widget::new();
        w.set_count(10);
        let g = w.create_gadget();
        assert_eq!(w.read_gadget(&g), 10);
    }

    #[test]
    fn method_with_mutable_handle_param() {
        init();
        let w = Widget::new();
        let mut g = w.create_gadget();
        assert_eq!(g.get(), 0);
        w.update_gadget(&mut g, 99);
        assert_eq!(g.get(), 99);
    }

    #[test]
    fn method_returning_handle() {
        init();
        let mut w = Widget::new();
        w.set_count(33);
        let g = w.create_gadget();
        assert_eq!(g.get(), 33);
    }

    #[test]
    fn method_returning_borrowed_handle() {
        init();
        let w = Widget::new();
        // Widget has an internal gadget with value 42.
        let g = w.gadget();
        assert_eq!(g.get(), 42);
        // g is an owned Gadget wrapper around a borrowed handle.
        // Dropping g calls ft_gadget_destroy, which deallocates the
        // shell without dropping the inner Gadget (still owned by w).
        drop(g);
        // Widget is still alive and usable.
        assert_eq!(w.name(), "widget");
    }

    // ================================================================
    // Builder pattern (by-value self -> Self)
    // ================================================================

    #[test]
    fn builder_method_returning_self() {
        init();
        let c = Config::new().set_name("myconfig").set_size(42);
        assert_eq!(c.get_name(), "myconfig");
        assert_eq!(c.get_size(), 42);
    }

    #[test]
    fn builder_method_returning_result_self_ok() {
        init();
        let c = Config::new().set_name("valid").validated().unwrap();
        assert_eq!(c.get_name(), "valid");
    }

    #[test]
    fn builder_method_returning_result_self_err() {
        init();
        let err = Config::new().validated().unwrap_err();
        assert!(matches!(err, TestError::InvalidInput(..)));
    }

    #[test]
    fn builder_consuming_self_returning_other_handle() {
        init();
        let mut b = GizmoBuilder::new();
        b.set_name("mygizmo");
        b.set_size(100);
        let g = b.build();
        assert_eq!(g.name(), "mygizmo");
        assert_eq!(g.size(), 100);
    }

    #[test]
    fn builder_consuming_self_returning_result_handle_ok() {
        init();
        let mut b = GizmoBuilder::new();
        b.set_name("valid");
        b.set_size(50);
        let g = b.try_build().unwrap();
        assert_eq!(g.name(), "valid");
        assert_eq!(g.size(), 50);
    }

    #[test]
    fn builder_consuming_self_returning_result_handle_err() {
        init();
        let b = GizmoBuilder::new();
        // name empty — try_build() should fail
        let err = b.try_build().unwrap_err();
        assert!(matches!(err, TestError::InvalidInput(..)));
    }

    // ================================================================
    // Error type
    // ================================================================

    #[test]
    fn error_display() {
        init();
        // Payloads use native Display; fieldless variants use static ABI messages.
        let w = Widget::new();
        // describe(99) → NotFound("code 99")
        let err = w.describe(99).unwrap_err();
        assert_eq!(format!("{err}"), "not found: code 99");
        // fail_always() → CustomMessage
        let err = w.fail_always().unwrap_err();
        assert_eq!(format!("{err}"), "custom error message");
        // fail_with_value() → InvalidInput
        let err = w.fail_with_value().unwrap_err();
        assert_eq!(format!("{err}"), "invalid input");
    }

    // ================================================================
    // Lifetime-parameterized types
    // ================================================================

    #[test]
    fn lifetime_type_borrowing_handle() {
        init();
        let mut w = Widget::new();
        w.set_count(77);
        let v = View::create(&w);
        assert_eq!(v.source_count(), 77);
    }

    #[test]
    fn lifetime_type_str_methods() {
        init();
        let w = Widget::new();
        let mut v = View::create(&w);
        assert_eq!(v.label(), "default");
        v.set_label("custom");
        assert_eq!(v.label(), "custom");
    }

    // ================================================================
    // Lifetime-parameterized trait impl
    // ================================================================

    #[test]
    fn lifetime_trait_impl_compiles() {
        init();
        let w = Widget::new();
        let v = View::create(&w);
        // Verifies impl<'a> Snapshot<'a> for View<'a> generated correctly
        let _: &dyn Snapshot = &v;
        // Verifies impl Snapshot<'static> for Widget generated correctly
        let _: &dyn Snapshot = &w;
    }

    // ================================================================
    // Vtable / exported traits
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
    // Free functions with fd types
    // ================================================================

    #[test]
    #[cfg(unix)]
    fn free_fn_clone_fd_via_rust_bindings() {
        init();
        use std::os::unix::io::{AsFd, AsRawFd};
        let stdout = std::io::stdout();
        let cloned = clone_fd(stdout.as_fd()).unwrap();
        assert!(cloned.as_raw_fd() >= 0);
        assert_ne!(cloned.as_raw_fd(), stdout.as_raw_fd());
    }

    // ================================================================
    // Destroy (implicit via Drop)
    // ================================================================

    #[test]
    fn destroy_via_drop() {
        init();
        let _w = Widget::new();
        // Drop runs ft_widget_destroy automatically
    }

    // ================================================================
    // Foreign handle types (cross-library FFI)
    // ================================================================

    #[cfg(windows)]
    #[test]
    fn foreign_windows_handle_is_available_only_on_its_target() {
        init();
        let widget = Widget::new();
        let value = ffier_test_foreign_lib_via_cdylib::ForeignWindows::new(41);
        assert_eq!(widget.read_windows_foreign(&value), 41);
    }

    #[test]
    fn foreign_apply_config() {
        init();
        let mut w = Widget::new();
        let config = ffier_test_foreign_lib_via_cdylib::ForeignConfig::new("imported", 99);
        apply_foreign_config(&mut w, &config);
        assert_eq!(w.name(), "imported");
        assert_eq!(w.get_count(), 99);
    }

    #[test]
    fn foreign_read_item_score() {
        init();
        let item = ffier_test_foreign_lib_via_cdylib::ForeignItem::new("test", 42);
        assert_eq!(read_foreign_item_score(&item), 42);
    }

    #[test]
    fn foreign_create_item() {
        init();
        let item = create_foreign_item("created", 7);
        assert_eq!(item.label(), "created");
        assert_eq!(item.score(), 7);
    }

    #[test]
    fn foreign_create_config_checked_ok() {
        init();
        let config = create_foreign_config_checked("valid", 55).unwrap();
        assert_eq!(config.name(), "valid");
        assert_eq!(config.value(), 55);
    }

    #[test]
    fn foreign_create_config_checked_err() {
        init();
        let err = create_foreign_config_checked("", 0).unwrap_err();
        assert!(matches!(err, TestError::NotFound(_)));
    }

    #[test]
    fn foreign_mut_param() {
        init();
        let mut item = ffier_test_foreign_lib_via_cdylib::ForeignItem::new("test", 10);
        double_foreign_item_score(&mut item);
        assert_eq!(item.score(), 20);
    }

    #[test]
    fn foreign_by_value_param() {
        init();
        let item = ffier_test_foreign_lib_via_cdylib::ForeignItem::new("consumed", 33);
        let score = consume_foreign_item(item);
        assert_eq!(score, 33);
        // `item` has been consumed — it's moved into the function
    }

    #[test]
    fn foreign_param_on_method() {
        init();
        let mut w = Widget::new();
        let config = ffier_test_foreign_lib_via_cdylib::ForeignConfig::new("method", 77);
        w.apply_config(&config);
        assert_eq!(w.name(), "method");
        assert_eq!(w.get_count(), 77);
    }

    #[test]
    fn foreign_lifetime_handle_param() {
        init();
        let text = String::from("borrowed");
        let value = ffier_test_foreign_lib_via_cdylib::ForeignBorrowed::new(&text);
        let w = Widget::new();
        assert_eq!(w.read_foreign_borrowed(&value), text.len() as i32);
    }
}
