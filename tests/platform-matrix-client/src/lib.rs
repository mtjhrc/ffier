#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

// Link the provider's Rust archive so strong extern calls resolve in this test.
use ffier_test_platform_matrix as _;

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(init_inner);
    }

    fn init_inner() {
        #[cfg(not(feature = "strong"))]
        {
            let library = unsafe { libloading::Library::new(env!("HANDLE_LIBRARY")) }.unwrap();
            #[cfg(unix)]
            let handle = libloading::os::unix::Library::from(library).into_raw();
            #[cfg(windows)]
            let handle = libloading::os::windows::Library::from(library).into_raw()
                as *mut core::ffi::c_void;
            unsafe { set_library_handle(core::ptr::NonNull::new(handle), LoadPolicy::Explicit) }
                .unwrap();
            let mut symbols = vec![
                Symbol::PlatformMatrixHandleApiRaw,
                Symbol::PlatformMatrixHandleApiEcho,
                Symbol::PlatformMatrixResource,
                Symbol::PlatformMatrixDirectResource,
                Symbol::PlatformMatrixFeatureWidth,
                Symbol::PlatformMatrixConditionalExport,
                Symbol::PlatformMatrixGeneratedValue,
                Symbol::PlatformMatrixHandleApiNew,
                Symbol::PlatformMatrixHandleApiFeatureWidth,
                Symbol::PlatformMatrixHandleApiThroughFeatureCallback,
                Symbol::PlatformMatrixFeatureConsumerWidth,
                Symbol::PlatformMatrixHandleApiDestroy,
                Symbol::PlatformMatrixHandleApiResource,
                Symbol::PlatformMatrixHandleApiOptionalRaw,
                Symbol::PlatformMatrixHandleApiTake,
                Symbol::PlatformMatrixHandleApiOptionalTake,
                Symbol::PlatformMatrixHandleApiThroughCallback,
                Symbol::PlatformMatrixHandleApiValue,
                Symbol::PlatformMatrixHandleConsumerNumber,
                Symbol::PlatformMatrixVtableHandleNew,
                Symbol::PlatformMatrixHandleApiFdNumber,
                Symbol::PlatformMatrixHandleApiFdNumberOptional,
                Symbol::PlatformMatrixHandleApiMaybeFd,
                Symbol::PlatformMatrixHandleApiDupFd,
                Symbol::PlatformMatrixHandleApiMaybeDupFd,
                Symbol::PlatformMatrixHandleApiMaybeOwnedFd,
                Symbol::PlatformMatrixCloneFd,
                Symbol::PlatformMatrixErrorDestroy,
            ];
            #[cfg(unix)]
            symbols.extend([
                Symbol::PlatformMatrixHandleApiSomethingWithConsole,
                Symbol::PlatformMatrixUnixValueNew,
                Symbol::PlatformMatrixUnixValueDestroy,
                Symbol::PlatformMatrixUnixValue,
            ]);
            #[cfg(windows)]
            symbols.extend([
                Symbol::PlatformMatrixHandleConsumerWindowsOnly,
                Symbol::PlatformMatrixHandleApiSomethingWithWindows,
                Symbol::PlatformMatrixWindowsValueNew,
                Symbol::PlatformMatrixWindowsValueDestroy,
                Symbol::PlatformMatrixWindowsValue,
            ]);
            require(&symbols).unwrap();
            #[cfg(feature = "wide")]
            require(&[Symbol::PlatformMatrixHandleApiWidth]).unwrap();
            #[cfg(unix)]
            assert!(!Symbol::PlatformMatrixWindowsValueNew.is_available());
            #[cfg(windows)]
            assert!(!Symbol::PlatformMatrixUnixValueNew.is_available());
        }
    }

    #[cfg(all(feature = "test-owner", not(feature = "strong")))]
    #[test]
    fn optional_owner_is_discovered_without_its_cargo_feature() {
        init();
        require(&[
            Symbol::PlatformMatrixOptionalApiNew,
            Symbol::PlatformMatrixOptionalApiDestroy,
            Symbol::PlatformMatrixOptionalApiResource,
            Symbol::PlatformMatrixHandleApiPlatformValue,
        ])
        .unwrap();
        let api = OptionalApi::default();
        assert!(!cfg!(feature = "owner-api"));
        assert_eq!(PlatformDefaults::platform_value(&HandleApi::new()), 77);
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::fd::{AsFd, AsRawFd};
            assert_eq!(api.resource(file.as_fd()), file.as_raw_fd());
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::{AsHandle, AsRawHandle};
            assert_eq!(api.resource(file.as_handle()), file.as_raw_handle());
        }
    }

    #[test]
    fn platform_signatures_and_callback_slots_work() {
        init();
        assert_eq!(conditional_export(), 77);
        assert_eq!(generated_value(), 1);
        #[cfg(feature = "wide")]
        {
            let input = 0x1_0000_007b_i64;
            let value: i64 = feature_width(input);
            assert_eq!(value, input + 8);
        }
        #[cfg(not(feature = "wide"))]
        {
            let value: i32 = feature_width(123);
            assert_eq!(value, 127);
        }
        let api = HandleApi::new();
        struct DefaultFeatureConsumer;
        impl FeatureConsumer for DefaultFeatureConsumer {}
        #[cfg(feature = "wide")]
        {
            let input = 0x1_0000_007b_i64;
            let value: i64 = api.feature_width(input);
            assert_eq!(value, input + 8);
            assert_eq!(FeatureConsumer::width(&api, input), input + 16);
            assert_eq!(
                FeatureConsumer::width(&DefaultFeatureConsumer, input),
                input + 8
            );
            assert_eq!(
                HandleApi::through_feature_callback(DefaultFeatureConsumer),
                131
            );
        }
        #[cfg(not(feature = "wide"))]
        {
            let value: i32 = api.feature_width(123);
            assert_eq!(value, 127);
            assert_eq!(FeatureConsumer::width(&api, 123), 127);
            assert_eq!(
                HandleApi::through_feature_callback(DefaultFeatureConsumer),
                127
            );
        }
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::fd::{AsFd, AsRawFd, OwnedFd};
            assert_eq!(HandleApi::raw(file.as_fd()), file.as_raw_fd());
            assert_eq!(resource(file.as_fd()), file.as_raw_fd());
            assert_eq!(direct_resource(file.as_fd()), file.as_raw_fd());
            assert_eq!(api.resource(file.as_fd()), file.as_raw_fd());
            assert_eq!(api.something_with_console(file.as_fd()), file.as_raw_fd());
            assert_eq!(
                HandleApi::echo(Some(file.as_fd())).unwrap().as_raw_fd(),
                file.as_raw_fd()
            );
            assert!(HandleApi::echo(None).is_none());
            assert_eq!(HandleApi::optional_raw(None), -1);
            assert_eq!(
                HandleApi::optional_raw(Some(file.as_fd())),
                file.as_raw_fd()
            );
            let owned: OwnedFd = file.try_clone().unwrap().into();
            let raw = owned.as_raw_fd();
            assert_eq!(HandleApi::take(owned).as_raw_fd(), raw);
            let owned: OwnedFd = file.try_clone().unwrap().into();
            let raw = owned.as_raw_fd();
            assert_eq!(
                HandleApi::optional_take(Some(owned)).unwrap().as_raw_fd(),
                raw
            );
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::{AsHandle, AsRawHandle, OwnedHandle};
            assert_eq!(HandleApi::raw(file.as_handle()), file.as_raw_handle());
            assert_eq!(resource(file.as_handle()), file.as_raw_handle());
            assert_eq!(direct_resource(file.as_handle()), file.as_raw_handle());
            assert_eq!(api.resource(file.as_handle()), file.as_raw_handle());
            assert_eq!(
                api.something_with_windows(file.as_handle()),
                file.as_raw_handle()
            );
            assert_eq!(
                HandleApi::echo(Some(file.as_handle()))
                    .unwrap()
                    .as_raw_handle(),
                file.as_raw_handle()
            );
            assert!(HandleApi::echo(None).is_none());
            assert!(HandleApi::optional_raw(None).is_null());
            assert_eq!(
                HandleApi::optional_raw(Some(file.as_handle())),
                file.as_raw_handle()
            );
            let owned: OwnedHandle = file.try_clone().unwrap().into();
            let raw = owned.as_raw_handle();
            assert_eq!(HandleApi::take(owned).as_raw_handle(), raw);
            let owned: OwnedHandle = file.try_clone().unwrap().into();
            let raw = owned.as_raw_handle();
            assert_eq!(
                HandleApi::optional_take(Some(owned))
                    .unwrap()
                    .as_raw_handle(),
                raw
            );
        }
        assert!(HandleApi::optional_take(None).is_none());
        // Resource conversions: same shapes the general suites used to cover,
        // selected per platform here instead of there.
        #[cfg(unix)]
        {
            use std::os::fd::{AsFd, AsRawFd};
            assert_eq!(api.fd_number(file.as_fd()), file.as_raw_fd());
            assert_eq!(api.fd_number_optional(Some(file.as_fd())), file.as_raw_fd());
            assert_eq!(api.fd_number_optional(None), -1);

            let some = api.maybe_fd(1).unwrap().unwrap();
            assert_eq!(some.as_raw_fd(), 0); // stdin
            assert!(api.maybe_fd(0).unwrap().is_none());
            assert!(api.maybe_fd(-1).is_err());

            let dup = api.dup_fd(file.as_fd());
            assert!(dup.as_raw_fd() >= 0);
            assert_ne!(dup.as_raw_fd(), file.as_raw_fd());

            let dup = api.maybe_dup_fd(Some(file.as_fd())).unwrap().unwrap();
            assert_ne!(dup.as_raw_fd(), file.as_raw_fd());
            assert!(api.maybe_dup_fd(None).unwrap().is_none());

            let owned = api.maybe_owned_fd(1).unwrap().unwrap();
            assert!(owned.as_raw_fd() >= 0);
            assert!(api.maybe_owned_fd(0).unwrap().is_none());
            assert!(api.maybe_owned_fd(-1).is_err());

            let cloned = clone_fd(file.as_fd()).unwrap();
            assert_ne!(cloned.as_raw_fd(), file.as_raw_fd());

            assert_eq!(HandleApi::raw(file.as_fd()), file.as_raw_fd());
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::{AsHandle, AsRawHandle};
            assert_eq!(
                api.fd_number(file.as_handle()),
                file.as_raw_handle() as isize
            );
            assert_eq!(
                api.fd_number_optional(Some(file.as_handle())),
                file.as_raw_handle() as isize
            );
            assert_eq!(api.fd_number_optional(None), 0);

            let some = api.maybe_fd(1).unwrap().unwrap();
            assert_eq!(some.as_raw_handle(), std::io::stdin().as_raw_handle());
            assert!(api.maybe_fd(0).unwrap().is_none());
            assert!(api.maybe_fd(-1).is_err());

            let dup = api.dup_fd(file.as_handle());
            assert!(!dup.as_raw_handle().is_null());
            assert_ne!(dup.as_raw_handle(), file.as_raw_handle());

            let dup = api.maybe_dup_fd(Some(file.as_handle())).unwrap().unwrap();
            assert_ne!(dup.as_raw_handle(), file.as_raw_handle());
            assert!(api.maybe_dup_fd(None).unwrap().is_none());

            let owned = api.maybe_owned_fd(1).unwrap().unwrap();
            assert!(!owned.as_raw_handle().is_null());
            assert!(api.maybe_owned_fd(0).unwrap().is_none());
            assert!(api.maybe_owned_fd(-1).is_err());

            let cloned = clone_fd(file.as_handle()).unwrap();
            assert_ne!(cloned.as_raw_handle(), file.as_raw_handle());

            assert_eq!(HandleApi::raw(file.as_handle()), file.as_raw_handle());
        }
        struct Callback;
        impl HandleConsumer for Callback {
            #[cfg(windows)]
            fn number(&self, handle: std::os::windows::io::BorrowedHandle<'_>) -> usize {
                use std::os::windows::io::AsRawHandle;
                handle.as_raw_handle() as usize
            }
        }
        assert!(HandleApi::through_callback(Callback) > 0);
        #[cfg(unix)]
        {
            assert_eq!(HandleApi::value(UnixValue::new()), 1);
            assert_eq!(unix_value(UnixValue::new()), 1);
        }
        #[cfg(windows)]
        {
            assert_eq!(HandleApi::value(WindowsValue::new()), 11);
            assert_eq!(windows_value(WindowsValue::new()), 11);
        }
        let vtable = Callback::__ffier_vtable();
        assert_eq!(vtable._num_slots, 2);
        assert_eq!(
            std::mem::size_of::<HandleConsumerVtable>(),
            std::mem::size_of::<ffier::VtableHeader>() + 3 * std::mem::size_of::<usize>()
        );
        #[cfg(unix)]
        assert!(vtable.__reserved_1.is_none());
        #[cfg(windows)]
        assert_eq!(
            unsafe { vtable.windows_only.unwrap()(&Callback as *const Callback as *mut _) },
            11
        );
    }
}

#[cfg(all(test, not(feature = "strong")))]
mod lazy_tests {
    mod api {
        include!(concat!(env!("OUT_DIR"), "/generated.rs"));
    }

    #[test]
    fn lazy_loading_pins_a_borrowed_native_library_handle() {
        let library = unsafe { libloading::Library::new(env!("HANDLE_LIBRARY")) }.unwrap();
        #[cfg(unix)]
        let library = libloading::os::unix::Library::from(library);
        #[cfg(windows)]
        let library = libloading::os::windows::Library::from(library);
        let raw = library.into_raw();
        let handle = core::ptr::NonNull::new(raw as *mut core::ffi::c_void).unwrap();
        // Retain ownership separately; the generated client borrows the handle.
        #[cfg(unix)]
        let library = unsafe { libloading::os::unix::Library::from_raw(raw) };
        #[cfg(windows)]
        let library = unsafe { libloading::os::windows::Library::from_raw(raw) };
        unsafe { api::set_library_handle(Some(handle), api::LoadPolicy::Lazy) }.unwrap();
        #[cfg(unix)]
        assert_eq!(api::HandleApi::optional_raw(None), -1);
        #[cfg(windows)]
        assert!(api::HandleApi::optional_raw(None).is_null());
        assert!(unsafe { api::set_library_handle(None, api::LoadPolicy::Explicit) }.is_err());
        // Keep the DLL loaded for this client's remaining static caches.
        std::mem::forget(library);
    }
}
