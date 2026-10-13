// Swap between native Rust linking and C ABI dynamic linking:
//   cargo test -p ffier-test-consumer --features native
//   cargo test -p ffier-test-consumer --no-default-features --features via-cdylib

#[cfg(all(test, unix, feature = "native"))]
use ffier_test_lib as api;

#[cfg(all(test, unix, feature = "via-cdylib"))]
use ffier_test_lib_via_cdylib as api;

#[cfg(all(test, unix))]
mod tests {
    use std::cell::Cell;
    use std::marker::PhantomData;
    use std::os::fd::AsRawFd;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::api::{
        self, Config, Gadget, Mixer, OptionalFlags, OptionalMode, OptionalWidget, OptionalWorker,
        View, Widget, optional_apply, optional_merge_flags, optional_mode_name, sum_gadget_values,
    };

    fn input_event(value: i32) -> api::InputEvent {
        api::InputEvent {
            device: api::InputDeviceIds {
                bus: 3,
                vendor: 0x1af4,
                product: 0x12,
                version: 1,
            },
            kind: api::InputEventKind::Key,
            code: 30,
            flags: api::InputEventFlags::REPEAT,
            value,
            pressure: 1.5,
            scale: 2.5,
        }
    }

    struct ThreadCallback<T> {
        value: T,
        drops: Arc<AtomicUsize>,
    }

    impl api::SendBackend for ThreadCallback<Cell<i32>> {
        fn step(&mut self) -> i32 {
            let value = self.value.get();
            self.value.set(value + 1);
            value
        }
    }

    impl api::ThreadBackend for ThreadCallback<i32> {
        fn value(&self) -> i32 {
            self.value
        }
    }

    impl<T> Drop for ThreadCallback<T> {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct SyncCallback {
        inner: ThreadCallback<i32>,
        not_send: PhantomData<Rc<()>>,
    }

    // SAFETY: Rc is never stored. Shared access touches only immutable data
    // and an atomic drop counter. The marker deliberately withholds Send.
    unsafe impl Sync for SyncCallback {}

    impl api::SyncBackend for SyncCallback {
        fn value(&self) -> i32 {
            self.inner.value
        }
    }

    #[test]
    fn opaque_and_value_types_preserve_verified_guarantees() {
        fn require_send<T: Send>() {}
        fn require_sync<T: Sync>() {}
        require_send::<api::Widget>();
        require_sync::<api::Widget>();
        require_send::<api::View<'_>>();
        require_sync::<api::View<'_>>();
        require_send::<api::SendCounter>();
        require_sync::<api::SyncCounter>();
        require_send::<api::BackendWorker<'_>>();
        require_send::<api::InputEvent>();
        require_sync::<api::InputEvent>();
        let mut widget = api::Widget::new();
        widget.set_count(19);
        let widget = std::thread::spawn(move || {
            assert_eq!(widget.get_count(), 19);
            widget
        })
        .join()
        .unwrap();
        let view = api::View::create(&widget);
        std::thread::scope(|scope| {
            scope.spawn(move || assert_eq!(view.source_count(), 19));
            scope.spawn(|| assert_eq!(widget.get_count(), 19));
        });
        let send = api::SendCounter::new(21);
        std::thread::spawn(move || assert_eq!(send.current_value(), 21))
            .join()
            .unwrap();
        let sync = api::SyncCounter::new(7);
        std::thread::scope(|scope| {
            scope.spawn(|| assert_eq!(sync.current_value(), 7));
            scope.spawn(|| assert_eq!(sync.current_value(), 7));
        });
        assert_eq!(api::run_send_backend(api::SendCounter::new(21)), 21);
        assert_eq!(api::share_sync_backend(&api::SyncCounter::new(7)), 14);
        assert_eq!(api::own_sync_backend(api::SyncCounter::new(8)), 16);
        assert_eq!(api::run_thread_backend(widget), 57);
    }

    #[test]
    fn owned_callback_transfers_and_destroys_on_worker_thread() {
        let drops = Arc::new(AtomicUsize::new(0));
        let callback = ThreadCallback {
            value: Cell::new(41),
            drops: drops.clone(),
        };
        let mut worker = api::BackendWorker::new(callback);
        assert_eq!(worker.step(), 41);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(std::thread::spawn(move || worker.run()).join().unwrap(), 42);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            api::run_send_backend(ThreadCallback {
                value: Cell::new(51),
                drops: drops.clone(),
            }),
            51
        );
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(
            api::run_thread_backend(ThreadCallback {
                value: 17,
                drops: drops.clone(),
            }),
            51
        );
        assert_eq!(drops.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn borrowed_thread_safe_callbacks_keep_ownership_and_lifetimes() {
        // Use a mutable borrow: Cell is Send but not Sync.
        struct MutableBackend<'a>(&'a mut Cell<i32>);
        impl api::SendBackend for MutableBackend<'_> {
            fn step(&mut self) -> i32 {
                self.0.get()
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let mut callback = ThreadCallback {
            value: Cell::new(10),
            drops: drops.clone(),
        };
        assert_eq!(api::borrow_send_backend(&mut callback), 10);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(callback);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let callback = SyncCallback {
            inner: ThreadCallback {
                value: 13,
                drops: drops.clone(),
            },
            not_send: PhantomData,
        };
        assert_eq!(api::share_sync_backend(&callback), 26);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(api::own_sync_backend(callback), 26);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        let mut value = Cell::new(81);
        let worker = api::BackendWorker::new(MutableBackend(&mut value));
        assert_eq!(worker.run(), 81);
        assert_eq!(value.get(), 81);
    }

    #[cfg(feature = "via-cdylib")]
    #[test]
    fn callback_handle_guarantees_match_ownership_and_borrow_kind() {
        use api::{SendBackend, SyncBackend, ThreadBackend};
        fn require_send<T: Send>(_: &T) {}
        fn require_sync<T: Sync>(_: &T) {}
        let drops = Arc::new(AtomicUsize::new(0));
        let handle = ThreadCallback {
            value: Cell::new(2),
            drops: drops.clone(),
        }
        .__into_callback_handle();
        require_send(&handle);
        std::thread::spawn(move || drop(handle)).join().unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let mut callback = ThreadCallback {
            value: Cell::new(3),
            drops: drops.clone(),
        };
        let handle = callback.__borrow_callback_handle_mut();
        require_send(&handle);
        std::thread::scope(|scope| {
            scope.spawn(move || drop(handle));
        });
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let callback = SyncCallback {
            inner: ThreadCallback {
                value: 4,
                drops: drops.clone(),
            },
            not_send: PhantomData,
        };
        let handle = callback.__borrow_callback_handle();
        require_send(&handle);
        require_sync(&handle);
        std::thread::scope(|scope| {
            scope.spawn(move || drop(handle));
        });
        let handle = callback.__into_callback_handle();
        require_sync(&handle);
        drop(handle);
        let handle = ThreadCallback {
            value: 5,
            drops: drops.clone(),
        }
        .__into_callback_handle();
        require_send(&handle);
        require_sync(&handle);
        std::thread::spawn(move || drop(handle)).join().unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 3);
    }

    struct BorrowedFruit<'a> {
        value: &'a i32,
        drops: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl api::Fruit for BorrowedFruit<'_> {
        fn value(&self) -> i32 {
            *self.value
        }
        fn try_count(&self, input: i32) -> Result<i32, api::TestError> {
            Ok(*self.value + input)
        }
        fn count_tags(&self, tags: &[&str]) -> i32 {
            tags.len() as i32
        }
    }

    impl Drop for BorrowedFruit<'_> {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    #[test]
    fn retained_owned_callback_lifetime_and_destruction() {
        let value = 42;
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        for mode in 0..3 {
            let fruit = BorrowedFruit {
                value: &value,
                drops: drops.clone(),
            };
            let retained = match mode {
                0 => api::RetainedFruit::new(fruit),
                1 => api::RetainedFruit::new_dynamic(fruit),
                _ => api::retain_fruit(fruit),
            };
            assert_eq!(retained.value(), 42);
            let before = drops.get();
            assert_eq!(retained.value(), 42);
            assert_eq!(drops.get(), before);
            drop(retained);
            assert_eq!(drops.get(), before + 1);
        }
        // An unbounded owned callback may borrow local data too.
        assert_eq!(
            api::Mixer::new()
                .add(BorrowedFruit {
                    value: &value,
                    drops: drops.clone()
                })
                .total(),
            42
        );
        assert_eq!(drops.get(), 4);
        static STATIC_VALUE: i32 = 91;
        let retained = api::RetainedFruit::new_static(BorrowedFruit {
            value: &STATIC_VALUE,
            drops: drops.clone(),
        });
        assert_eq!(retained.value(), 91);
        assert_eq!(drops.get(), 4);
        drop(retained);
        assert_eq!(drops.get(), 5);
    }

    #[test]
    fn borrowed_callbacks_do_not_transfer_ownership() {
        let value = 17;
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut fruit = BorrowedFruit {
            value: &value,
            drops: drops.clone(),
        };
        let retained = api::RetainedFruit::new_static(api::Apple::new(1));
        assert_eq!(retained.borrowed_value(&fruit), 17);
        assert_eq!(retained.borrowed_value_dynamic(&fruit), 17);
        assert_eq!(retained.borrowed_mut_value(&mut fruit), 17);
        assert_eq!(drops.get(), 0);
        drop(fruit);
        assert_eq!(drops.get(), 1);
        let known = api::Apple::new(23);
        assert_eq!(retained.borrowed_value(&known), 23);
        assert_eq!(retained.borrowed_value_dynamic(&known), 23);
    }

    #[cfg(feature = "via-cdylib")]
    #[test]
    fn untransferred_callback_handle_destroys_value_once() {
        use api::Fruit;
        let value = 9;
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let handle = BorrowedFruit {
            value: &value,
            drops: drops.clone(),
        }
        .__into_callback_handle();
        assert_eq!(drops.get(), 0);
        drop(handle);
        assert_eq!(drops.get(), 1);
        drop(api::Apple::new(1).__into_callback_handle());
    }

    #[test]
    fn test_value_struct_rejects_invalid_enum() {
        use api::FfiType as _;

        let mut raw = input_event(0).into_c();
        raw.kind = 999;
        assert!(
            std::panic::catch_unwind(|| unsafe { <api::InputEvent as api::FfiType>::from_c(raw) })
                .is_err()
        );
    }

    #[test]
    fn test_value_struct_roundtrip_and_references() {
        use api::InputSource as _;

        let mut queue = api::EventQueue::new();
        let mut event = input_event(19);
        event.flags = api::InputEventFlags::from_bits_retain(0x8000_0001);
        assert_eq!(queue.echo_event(event), event);
        assert_eq!(queue.peek_ids().vendor, 0x1af4);
        assert!(queue.maybe_peek_ids(false).is_none());
        assert_eq!(queue.maybe_peek_ids(true).unwrap().product, 0x12);

        let mut ids = event.device;
        ids.vendor = 2;
        queue.set_ids(&ids);
        assert_eq!(queue.peek_ids().vendor, 2);
        queue.edit_ids().vendor = 5;
        queue.copy_ids_to(&mut ids);
        assert_eq!(ids.vendor, 5);
        assert_eq!(queue.optional_ids_vendor(None), 0);
        assert_eq!(queue.optional_ids_vendor(Some(&ids)), 5);
        queue.optional_ids_mut(None);
        ids.vendor = 0;
        queue.optional_ids_mut(Some(&mut ids));
        assert_eq!(ids.vendor, 5);
        assert_eq!(queue.try_peek_ids(true).unwrap().vendor, 5);
        assert!(queue.try_peek_ids(false).is_err());
        queue.try_edit_ids(true).unwrap().vendor = 7;
        assert!(queue.try_edit_ids(false).is_err());
        assert_eq!(queue.peek_ids().vendor, 7);
        assert_eq!(api::borrow_input_ids(&ids).vendor, 5);
        api::borrow_input_ids_mut(&mut ids).vendor = 6;
        assert_eq!(ids.vendor, 6);
    }

    #[test]
    fn test_optional_value_and_result_paths() {
        use api::{InputBackend as _, InputSource as _};

        let mut queue = api::EventQueue::new();
        assert_eq!(queue.pop_event().unwrap().value, 42);
        assert!(queue.pop_event().is_none());

        let mut queue = api::EventQueue::new();
        assert_eq!(queue.poll_event().unwrap().value, 42);
        assert!(queue.poll_event().is_none());

        let mut queue = api::EventQueue::new();
        assert_eq!(queue.event_or_error().unwrap().value, 42);
        assert!(queue.event_or_error().is_err());

        let mut queue = api::EventQueue::new();
        assert_eq!(queue.next_event(false).unwrap().unwrap().value, 42);
        assert!(queue.next_event(false).unwrap().is_none());
        assert!(matches!(
            queue.next_event(true),
            Err(api::TestError::InvalidInput(..))
        ));
    }

    struct CallbackBackend {
        ids: api::InputDeviceIds,
        event: Option<api::InputEvent>,
    }

    impl api::InputBackend for CallbackBackend {
        fn current_ids(&self) -> Result<api::InputDeviceIds, api::TestError> {
            Ok(self.ids)
        }

        fn peek_ids_result(&self) -> Result<&api::InputDeviceIds, api::TestError> {
            Ok(&self.ids)
        }

        fn edit_ids_result(&mut self) -> Result<&mut api::InputDeviceIds, api::TestError> {
            Ok(&mut self.ids)
        }

        fn next_event(&mut self, fail: bool) -> Result<Option<api::InputEvent>, api::TestError> {
            if fail {
                Err(api::TestError::InvalidInput())
            } else {
                Ok(self.event.take())
            }
        }

        fn fill_config<'a>(&self, output: &'a mut [u8]) -> Result<&'a mut [u8], api::TestError> {
            let len = output.len().min(3);
            output[..len].copy_from_slice(&[1, 2, 3][..len]);
            Ok(&mut output[..len])
        }
    }

    struct CallbackSource {
        events: Vec<api::InputEvent>,
        ids: api::InputDeviceIds,
    }

    impl api::InputSource for CallbackSource {
        fn poll_event(&mut self) -> Option<api::InputEvent> {
            self.events.pop()
        }

        fn device_ids(&self) -> api::InputDeviceIds {
            self.ids
        }

        fn peek_ids(&self) -> &api::InputDeviceIds {
            &self.ids
        }

        fn edit_ids(&mut self) -> &mut api::InputDeviceIds {
            &mut self.ids
        }

        fn maybe_peek_ids(&self, available: bool) -> Option<&api::InputDeviceIds> {
            available.then_some(&self.ids)
        }

        fn submit_event(&mut self, event: api::InputEvent) {
            self.events.push(event);
        }
    }

    #[test]
    fn test_value_struct_vtable_callbacks() {
        let event = input_event(42);
        let backend = CallbackBackend {
            ids: event.device,
            event: Some(event),
        };
        assert_eq!(api::probe_input_backend_values(backend).unwrap(), 20701);

        let mut backend = CallbackBackend {
            event: Some(event),
            ids: event.device,
        };
        assert_eq!(
            api::probe_borrowed_input_backend_values(&mut backend).unwrap(),
            20701
        );
        assert_eq!(backend.ids.vendor, 0x1af5);

        let source = CallbackSource {
            events: Vec::new(),
            ids: event.device,
        };
        assert_eq!(api::probe_input_source(source), 13809);
    }

    fn make_widget() -> Widget {
        Widget::new()
    }

    fn make_named_widget(name: &str) -> Widget {
        Widget::with_name(name)
    }

    fn widget_roundtrip() -> i32 {
        let mut w = make_widget();
        w.set_count(42);
        w.get_count()
    }

    fn builder_chain() -> (String, i32) {
        let c = Config::new().set_name("hello").set_size(99);
        (c.get_name().to_owned(), c.get_size())
    }

    fn result_ok_path() -> Result<i32, api::TestError> {
        let w = make_widget();
        w.parse_count("test")
    }

    fn result_err_path() -> Result<(), api::TestError> {
        let w = make_widget();
        w.fail_always()
    }

    fn handle_param_roundtrip() -> i32 {
        let mut w = make_widget();
        w.set_count(7);
        let g: Gadget = w.create_gadget();
        w.read_gadget(&g)
    }

    #[test]
    fn test_constructor() {
        let w = make_widget();
        assert_eq!(w.get_count(), 0);
    }

    #[test]
    fn test_named_constructor() {
        let w = make_named_widget("hello");
        assert_eq!(w.name(), "hello");
    }

    #[test]
    fn test_roundtrip() {
        assert_eq!(widget_roundtrip(), 42);
    }

    #[test]
    fn test_mut_self_return_chaining() {
        let mut w = Widget::new();
        w.with_count(10).with_count(20);
        assert_eq!(w.get_count(), 20);
    }

    #[test]
    fn test_builder() {
        let (name, size) = builder_chain();
        assert_eq!(name, "hello");
        assert_eq!(size, 99);
    }

    #[test]
    fn test_result_ok() {
        assert_eq!(result_ok_path().unwrap(), 4); // len("test")
    }

    #[test]
    fn test_result_err() {
        let err = result_err_path().unwrap_err();
        assert!(matches!(err, api::TestError::CustomMessage));
    }

    #[test]
    fn test_handle_param() {
        assert_eq!(handle_param_roundtrip(), 7);
    }

    #[test]
    fn test_str_return() {
        let w = make_widget();
        assert_eq!(w.name(), "widget");
        assert_eq!(w.echo("ping"), "ping");
    }

    #[test]
    fn test_bytes_return() {
        let w = make_named_widget("abc");
        assert_eq!(w.data(), b"abc");
    }

    #[test]
    fn test_bool_return() {
        let w = make_widget();
        assert!(w.is_active());
    }

    #[test]
    fn test_str_slice_param() {
        let mut w = make_widget();
        w.set_tags(&["a", "b", "c"]);
        assert_eq!(w.tags_joined(), "a,b,c");
    }

    #[test]
    fn test_primitive_slice_param() {
        let w = make_widget();
        assert_eq!(w.sum_values(&[10, 20, 30]), 60);
        assert_eq!(w.sum_values(&[]), 0);
    }

    #[test]
    fn test_handle_slice_param() {
        let mut w = make_widget();
        w.set_count(5);
        let g1 = w.create_gadget();
        let g2 = w.create_gadget();
        w.set_count(7);
        let g3 = w.create_gadget();
        assert_eq!(w.sum_gadgets(&[&g1, &g2, &g3]), 17);
        assert_eq!(w.sum_gadgets(&[]), 0);
    }

    #[test]
    fn test_handle_slice_param_free_fn() {
        let mut w = make_widget();
        w.set_count(3);
        let g1 = w.create_gadget();
        w.set_count(4);
        let g2 = w.create_gadget();
        assert_eq!(sum_gadget_values(&[&g1, &g2]), 7);
    }

    #[test]
    fn test_result_str() {
        let w = make_widget();
        assert_eq!(w.describe(0).unwrap(), "zero");
        assert!(w.describe(99).is_err());
    }

    #[test]
    fn test_mutable_handle_param() {
        let w = make_widget();
        let mut g = w.create_gadget();
        w.update_gadget(&mut g, 123);
        assert_eq!(g.get(), 123);
    }

    #[test]
    fn test_builder_validated_ok() {
        let c = Config::new().set_name("valid").validated().unwrap();
        assert_eq!(c.get_name(), "valid");
    }

    #[test]
    fn test_builder_validated_err() {
        let result = Config::new().validated();
        assert!(result.is_err());
    }

    // ================================================================
    // Lifetime-parameterized types (View<'a> borrows Widget)
    // ================================================================

    #[test]
    fn test_view_borrows_widget() {
        let mut w = Widget::new();
        w.set_count(77);
        let v = View::create(&w);
        assert_eq!(v.source_count(), 77);
    }

    #[test]
    fn test_view_label() {
        let w = Widget::new();
        let mut v = View::create(&w);
        assert_eq!(v.label(), "default");
        v.set_label("custom");
        assert_eq!(v.label(), "custom");
    }

    #[test]
    fn test_view_lifetime_enforced() {
        // This should compile: view lives shorter than widget
        let mut w = Widget::new();
        w.set_count(42);
        let count = {
            let v = View::create(&w);
            v.source_count()
        };
        assert_eq!(count, 42);
    }

    // ================================================================
    // Snapshot trait — generic lifetime impl for non-lifetime struct
    // ================================================================

    #[test]
    fn test_gadget_snapshot_trait() {
        use api::Snapshot;
        let mut w = Widget::new();
        w.set_count(42);
        let g = w.create_gadget();
        // Gadget implements Snapshot<'a> despite having no lifetime params itself.
        // This verifies that the generated impl block doesn't add a spurious
        // lifetime parameter to the Gadget struct.
        assert_eq!(g.snap_description(), "gadget");
        assert_eq!(g.snap_source_count(), 42);
        let view = View::create(&w);
        assert_eq!(view.snapshot_count(g), 42);
        assert_eq!(view.snapshot_count(View::create(&w)), 42);
    }

    // ================================================================
    // Mixer + custom Fruit type
    // ================================================================

    struct Banana {
        sweetness: i32,
    }

    impl api::Fruit for Banana {
        fn value(&self) -> i32 {
            self.sweetness
        }
        fn try_count(&self, input: i32) -> Result<i32, api::TestError> {
            if input < 0 {
                return Err(api::TestError::InvalidInput());
            }
            Ok(self.sweetness + input)
        }
        fn count_tags(&self, tags: &[&str]) -> i32 {
            tags.len() as i32
        }
    }

    #[test]
    fn test_mixer_with_known_types() {
        let m = Mixer::new()
            .add(api::Apple::new(5))
            .add(api::Orange::new(3));
        assert_eq!(m.total(), 8);
    }

    #[test]
    fn test_mixer_with_custom_banana() {
        let m = Mixer::new()
            .add(Banana { sweetness: 10 })
            .add(api::Apple::new(5));
        assert_eq!(m.total(), 15);
    }

    // ================================================================
    // Vtable default method probe trampoline tests
    // ================================================================

    #[test]
    fn test_custom_type_default_label_via_ffi() {
        // Banana doesn't override label() — the probe trampoline should
        // detect FfierDefaultMarker, patch the vtable to None, and the
        // library should use its default impl ("fruit").
        //
        // fruit_label_len calls fruit.label().len() on the library side.
        // "fruit".len() = 5.
        let m = Mixer::new();
        assert_eq!(m.fruit_label_len(Banana { sweetness: 42 }), 5);
    }

    #[test]
    fn test_known_type_default_label_via_ffi() {
        // Apple also doesn't override label — but it's a known type,
        // so it uses the self-dispatch path (ft_fruit_label).
        let m = Mixer::new();
        assert_eq!(m.fruit_label_len(api::Apple::new(10)), 5);
    }

    // ================================================================
    // Dispatch path verification (via-cdylib only)
    // ================================================================

    #[cfg(feature = "via-cdylib")]
    mod dispatch_tests {
        use super::*;

        /// Peek at a handle's dispatch kind. Consumes and destroys the handle.
        fn dispatch_kind_of(fruit: impl api::Fruit) -> String {
            // The next FFI call consumes this handle and runs its destructor.
            let handle = unsafe { fruit.__into_raw_handle() };
            let kind = unsafe { api::ft_debug_fruit_dispatch_kind(handle) };
            unsafe { kind.as_str_unchecked() }.to_owned()
        }

        #[test]
        fn apple_dispatches_directly() {
            assert_eq!(dispatch_kind_of(api::Apple::new(5)), "Apple");
        }

        #[test]
        fn orange_dispatches_directly() {
            assert_eq!(dispatch_kind_of(api::Orange::new(3)), "Orange");
        }

        #[test]
        fn banana_dispatches_via_vtable() {
            assert_eq!(dispatch_kind_of(Banana { sweetness: 7 }), "VtableFruit");
        }
    }

    #[test]
    fn test_bitflags_roundtrip() {
        let w = make_widget();
        let perms = w.add_permission(api::Permissions::READ, api::Permissions::WRITE);
        assert_eq!(perms, api::Permissions::READ | api::Permissions::WRITE);
    }

    #[test]
    fn test_bitflags_single_flag() {
        let w = make_widget();
        let perms = w.add_permission(api::Permissions::empty(), api::Permissions::EXECUTE);
        assert_eq!(perms, api::Permissions::EXECUTE);
    }

    struct Triple;

    impl OptionalWorker for Triple {
        fn amplify(&self, input: i32) -> i32 {
            input * 3
        }
    }

    #[test]
    fn test_optional_handle_and_trait_impl() {
        let widget = OptionalWidget::new(7);
        assert_eq!(widget.base(), 7);
        assert_eq!(widget.amplify(3), 21);
        assert_eq!(optional_apply(widget, 4), 28);
    }

    #[test]
    fn test_optional_custom_trait_vtable_dispatch() {
        assert_eq!(optional_apply(Triple, 5), 15);
    }

    #[test]
    fn test_optional_enum_and_bitflags() {
        assert_eq!(optional_mode_name(OptionalMode::Basic), "basic");
        let flags = optional_merge_flags(OptionalFlags::READ, OptionalFlags::WRITE);
        assert_eq!(flags, OptionalFlags::READ | OptionalFlags::WRITE);
    }

    #[test]
    fn test_maybe_fd_some() {
        let mut w = make_widget();
        w.set_count(1);
        let result = w.maybe_fd(1);
        let fd = result.unwrap().unwrap();
        // Should be stdin (fd 0)
        assert_eq!(fd.as_raw_fd(), 0);
    }

    #[test]
    fn test_maybe_fd_none() {
        let w = make_widget();
        let result = w.maybe_fd(0);
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_maybe_fd_error() {
        let w = make_widget();
        let result = w.maybe_fd(-1);
        assert!(matches!(
            result.unwrap_err(),
            api::TestError::InvalidInput(..)
        ));
    }

    #[test]
    fn test_maybe_owned_fd_some() {
        let w = make_widget();
        let result = w.maybe_owned_fd(1);
        let fd = result.unwrap().unwrap();
        // Should be a dup of stdin — valid fd, not fd 0 itself
        assert!(fd.as_raw_fd() >= 0);
    }

    #[test]
    fn test_maybe_owned_fd_none() {
        let w = make_widget();
        let result = w.maybe_owned_fd(0);
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_maybe_owned_fd_error() {
        let w = make_widget();
        let result = w.maybe_owned_fd(-1);
        assert!(matches!(
            result.unwrap_err(),
            api::TestError::InvalidInput(..)
        ));
    }

    #[test]
    fn test_maybe_dup_fd_some() {
        use std::os::fd::BorrowedFd;
        let w = make_widget();
        // Pass stdin as Some
        let stdin = unsafe { BorrowedFd::borrow_raw(0) };
        let result = w.maybe_dup_fd(Some(stdin));
        let fd = result.unwrap().unwrap();
        assert!(fd.as_raw_fd() >= 0);
        assert_ne!(fd.as_raw_fd(), 0); // should be a new fd
    }

    #[test]
    fn test_maybe_dup_fd_none() {
        let w = make_widget();
        let result = w.maybe_dup_fd(None);
        assert!(result.unwrap().is_none());
    }

    // -----------------------------------------------------------------------
    // ForeignSlice / &[T] handle array returns
    // -----------------------------------------------------------------------

    #[test]
    fn test_gadgets_slice_len() {
        let w = make_widget();
        let gadgets = w.gadgets();
        assert_eq!(gadgets.len(), 2);
        assert!(!gadgets.is_empty());
    }

    #[test]
    fn test_gadgets_slice_index() {
        let w = make_widget();
        let gadgets = w.gadgets();
        assert_eq!(gadgets[0].get(), 10);
        assert_eq!(gadgets[1].get(), 20);
    }

    #[test]
    fn test_gadgets_slice_iter() {
        let w = make_widget();
        let gadgets = w.gadgets();
        let sum: i32 = gadgets.iter().map(|g| g.get()).sum();
        assert_eq!(sum, 30);
    }

    #[test]
    fn test_gadgets_slice_for_loop() {
        let w = make_widget();
        let gadgets = w.gadgets();
        let mut values = Vec::new();
        for g in gadgets.iter() {
            values.push(g.get());
        }
        assert_eq!(values, vec![10, 20]);
    }
}
