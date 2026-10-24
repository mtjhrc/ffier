#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

// Link the provider's Rust archive so strong extern calls resolve in this test.
use ffier_test_windows_handles as _;

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
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
                Symbol::WindowsHandlesHandleApiRaw,
                Symbol::WindowsHandlesHandleApiEcho,
                Symbol::WindowsHandlesResource,
                Symbol::WindowsHandlesDirectResource,
                Symbol::WindowsHandlesHandleApiNew,
                Symbol::WindowsHandlesHandleApiDestroy,
                Symbol::WindowsHandlesHandleApiResource,
                Symbol::WindowsHandlesHandleApiOptionalRaw,
                Symbol::WindowsHandlesHandleApiTake,
                Symbol::WindowsHandlesHandleApiOptionalTake,
                Symbol::WindowsHandlesHandleApiThroughCallback,
                Symbol::WindowsHandlesHandleApiValue,
                Symbol::WindowsHandlesHandleConsumerNumber,
                Symbol::WindowsHandlesVtableHandleNew,
            ];
            #[cfg(unix)]
            symbols.extend([
                Symbol::WindowsHandlesHandleApiSomethingWithConsole,
                Symbol::WindowsHandlesUnixValueNew,
                Symbol::WindowsHandlesUnixValueDestroy,
                Symbol::WindowsHandlesUnixValue,
            ]);
            #[cfg(windows)]
            symbols.extend([
                Symbol::WindowsHandlesHandleConsumerWindowsOnly,
                Symbol::WindowsHandlesHandleApiSomethingWithWindows,
                Symbol::WindowsHandlesWindowsValueNew,
                Symbol::WindowsHandlesWindowsValueDestroy,
                Symbol::WindowsHandlesWindowsValue,
            ]);
            require(&symbols).unwrap();
            #[cfg(unix)]
            assert!(!Symbol::WindowsHandlesWindowsValueNew.is_available());
            #[cfg(windows)]
            assert!(!Symbol::WindowsHandlesUnixValueNew.is_available());
        }
    }

    #[test]
    fn platform_signatures_and_callback_slots_work() {
        init();
        let api = HandleApi::new();
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
