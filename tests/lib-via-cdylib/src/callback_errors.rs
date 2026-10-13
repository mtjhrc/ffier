include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../callback_error_client.rs"
));

#[test]
fn fieldless_values_allocate_only_for_ffi_and_formatting() {
    init();
    let before = api::test_error_drop_count();
    let invalid = api::TestError::InvalidInput();
    let custom = api::TestError::CustomMessage;
    assert_eq!(api::test_error_drop_count(), before);
    assert!(matches!(invalid, api::TestError::InvalidInput(..)));
    assert_eq!(invalid.to_string(), "invalid input");
    assert_eq!(api::test_error_drop_count(), before + 1);
    assert!(matches!(custom, api::TestError::CustomMessage));
    assert_eq!(custom.to_string(), "custom error message");
    // The error type named Error must also support construction.
    assert_eq!(api::Error::Failed.to_string(), "generic failure");
    assert_eq!(api::test_error_drop_count(), before + 2);
    drop(invalid);
    drop(custom);
    // Client values have no native handle to destroy; formatting already
    // destroyed each temporary native error exactly once.
    assert_eq!(api::test_error_drop_count(), before + 2);
}

#[test]
fn callback_created_error_reconstructs_native_variant_and_returns_to_client() {
    init();
    let before = api::test_error_drop_count();
    let drops = Rc::new(Cell::new(0));
    let error =
        api::roundtrip_input_backend_error(FailingBackend(drops.clone(), None)).unwrap_err();
    // The fieldless native error is destroyed when received as a client value.
    assert_eq!(api::test_error_drop_count(), before + 1);
    assert!(matches!(error, api::TestError::InvalidInput(..)));
    assert_eq!(error.to_string(), "invalid input");
    assert_eq!(drops.get(), 1);
    // Formatting creates and destroys a separate temporary native error.
    assert_eq!(api::test_error_drop_count(), before + 2);
    drop(error);
    assert_eq!(api::test_error_drop_count(), before + 2);
}

#[test]
fn payload_error_forwarding_retains_native_handle_and_payload() {
    init();
    let before = api::test_error_drop_count();
    let error = api::Widget::new().fail_with_number(76).unwrap_err();
    let api::TestError::NumericError(ref data) = error else {
        panic!("expected NumericError")
    };
    assert_eq!(data.field_0(), 76);
    assert_eq!(error.to_string(), "numeric error: 76");
    assert_eq!(api::test_error_drop_count(), before);
    let drops = Rc::new(Cell::new(0));
    let error =
        api::roundtrip_input_backend_error(FailingBackend(drops.clone(), Some(error))).unwrap_err();
    let api::TestError::NumericError(ref data) = error else {
        panic!("expected NumericError")
    };
    assert_eq!(data.field_0(), 76);
    assert_eq!(error.to_string(), "numeric error: 76");
    assert_eq!(drops.get(), 1);
    assert_eq!(api::test_error_drop_count(), before);
    drop(error);
    assert_eq!(api::test_error_drop_count(), before + 1);
}

#[test]
fn client_constructs_string_payload_including_non_exhaustive_native_variant() {
    init();
    let before = api::test_error_drop_count();
    let error = api::TestError::NotFound(Box::<str>::from("client message").into());
    assert_eq!(error.to_string(), "not found: client message");
    let error =
        api::roundtrip_input_backend_error(FailingBackend(Rc::new(Cell::new(0)), Some(error)))
            .unwrap_err();
    let api::TestError::NotFound(ref data) = error else {
        panic!("expected NotFound")
    };
    assert_eq!(data.field_0(), "client message");
    assert_eq!(api::test_error_drop_count(), before);
    drop(error);
    assert_eq!(api::test_error_drop_count(), before + 1);
}

#[test]
fn client_constructs_multiple_fields_and_borrows_handles_without_consuming_them() {
    init();
    let before = api::test_error_drop_count();
    let gadgets_before = api::gadget_drop_count();
    let ids = api::InputDeviceIds {
        bus: 3,
        vendor: 76,
        product: 1,
        version: 1,
    };
    let error = api::TestError::ContextError(api::TestErrorContextErrorData::new(
        76,
        "client message".into(),
        api::Gadget::new(76),
        ids,
        api::LogLevel::Warn,
        api::Permissions::READ | api::Permissions::WRITE,
    ));
    assert_eq!(api::gadget_drop_count(), gadgets_before);
    assert_eq!(error.to_string(), "context error: 76, client message");
    let error =
        api::roundtrip_input_backend_error(FailingBackend(Rc::new(Cell::new(0)), Some(error)))
            .unwrap_err();
    let api::TestError::ContextError(ref data) = error else {
        panic!("expected ContextError")
    };
    assert_eq!(data.field_0(), 76);
    assert_eq!(data.field_1(), "client message");
    let gadget = data.field_2();
    assert_eq!(gadget.get(), 76);
    drop(gadget);
    assert_eq!(api::gadget_drop_count(), gadgets_before);
    // Repeated getters allocate/destroy borrowed shells, retaining the field.
    assert_eq!(data.field_2().get(), 76);
    assert_eq!(data.field_3().vendor, 76);
    assert!(matches!(data.field_4(), api::LogLevel::Warn));
    assert_eq!(
        data.field_5(),
        api::Permissions::READ | api::Permissions::WRITE
    );
    assert_eq!(api::test_error_drop_count(), before);
    drop(error);
    assert_eq!(api::test_error_drop_count(), before + 1);
    assert_eq!(api::gadget_drop_count(), gadgets_before + 1);
}

#[test]
fn owned_file_payload_is_borrowed_for_inspection_and_closed_with_error() {
    use std::io::Read;
    use std::os::fd::{AsRawFd, OwnedFd};
    init();
    let (stream, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
    peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let fd: OwnedFd = stream.into();
    let raw = fd.as_raw_fd();
    let error = api::TestError::FileError(fd.into());
    let error =
        api::roundtrip_input_backend_error(FailingBackend(Rc::new(Cell::new(0)), Some(error)))
            .unwrap_err();
    let api::TestError::FileError(ref data) = error else {
        panic!("expected FileError")
    };
    assert_eq!(data.field_0().as_raw_fd(), raw);
    assert_eq!(data.field_0().as_raw_fd(), raw);
    let duplicate = data.field_0().try_clone_to_owned().unwrap();
    drop(duplicate);
    drop(error);
    // EOF proves the native error closed the descriptor (without relying on
    // a descriptor number that another parallel test might have reused).
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
}

#[test]
fn callback_errors_leave_success_outputs_untouched() {
    init();
    let before = api::test_error_drop_count();
    let drops = Rc::new(Cell::new(0));
    let callback = FailingBackend(drops.clone(), None).__into_callback_handle();
    unsafe {
        let handle = &*(callback.as_raw() as *const ffier::FfierHandle<ffier::VtableHandle>);
        let vtable = &*(handle.value.vtable_ptr as *const api::InputBackendVtable);
        let data = handle.value.user_data as *mut core::ffi::c_void;
        let mut error = core::ptr::null_mut();
        let mut ids = api::__FfierCReprInputDeviceIds {
            bus: 1,
            vendor: 0xa5a5,
            product: 0x5a5a,
            version: 2,
        };
        let result = vtable.current_ids.unwrap()(data, &mut ids, &mut error);
        assert_ne!(result, 0);
        assert_eq!((ids.vendor, ids.product), (0xa5a5, 0x5a5a));
        assert_eq!(
            api::TestError::from_ffi(result, error).to_string(),
            "custom error message"
        );

        let mut ids_ptr = &ids as *const _;
        error = core::ptr::null_mut();
        let result = vtable.peek_ids_result.unwrap()(data, &mut ids_ptr, &mut error);
        assert_ne!(result, 0);
        assert!(core::ptr::eq(ids_ptr, &ids));
        drop(api::TestError::from_ffi(result, error));

        let mut ids_ptr = &mut ids as *mut _;
        let original_ptr = ids_ptr;
        error = core::ptr::null_mut();
        let result = vtable.edit_ids_result.unwrap()(data, &mut ids_ptr, &mut error);
        assert_ne!(result, 0);
        assert_eq!(ids_ptr, original_ptr);
        drop(api::TestError::from_ffi(result, error));

        // Both flag values detect accidental writes to the presence output.
        for sentinel in [false, true] {
            let mut present = sentinel;
            let mut event = api::__FfierCReprInputEvent {
                device: api::__FfierCReprInputDeviceIds {
                    bus: 1,
                    vendor: 0xa5a5,
                    product: 0x5a5a,
                    version: 2,
                },
                kind: 0xa5,
                code: 0x5a,
                flags: 0x1234,
                value: -123,
                pressure: 0.25,
                scale: 1.5,
            };
            error = core::ptr::null_mut();
            let result =
                vtable.next_event.unwrap()(data, true, &mut present, &mut event, &mut error);
            assert_ne!(result, 0);
            assert_eq!(present, sentinel);
            assert_eq!(
                (event.device.vendor, event.device.product),
                (0xa5a5, 0x5a5a)
            );
            assert_eq!((event.device.bus, event.device.version), (1, 2));
            assert_eq!(
                (event.kind, event.code, event.flags, event.value),
                (0xa5, 0x5a, 0x1234, -123)
            );
            assert_eq!((event.pressure, event.scale), (0.25, 1.5));
            let error = api::TestError::from_ffi(result, error);
            assert!(matches!(error, api::TestError::InvalidInput(..)));
            drop(error);
        }

        let mut bytes = [0xa5; 4];
        let mut output = ffier::FfierMutBytes {
            data: bytes.as_mut_ptr(),
            len: bytes.len(),
        };
        let original_ptr = output.data;
        error = core::ptr::null_mut();
        let result = vtable.fill_config.unwrap()(data, output, &mut output, &mut error);
        assert_ne!(result, 0);
        assert_eq!((output.data, output.len), (original_ptr, 4));
        assert_eq!(bytes, [0xa5; 4]);
        drop(api::TestError::from_ffi(result, error));
    }
    // Six callback errors and one separate formatting temporary were destroyed.
    assert_eq!(api::test_error_drop_count(), before + 7);
    assert_eq!(drops.get(), 0);
    drop(callback);
    assert_eq!(drops.get(), 1);
}
