#![cfg(any(feature = "ffier-dynlookup", feature = "ffier-strong"))]
#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::fd::{AsFd, AsRawFd};
    #[cfg(windows)]
    use std::os::windows::io::{AsHandle, AsRawHandle};

    fn init() {
        #[cfg(not(feature = "ffier-strong"))]
        {
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| unsafe {
                #[cfg(windows)]
                let library =
                    libloading::os::windows::Library::new(env!("FIXTURE_LIBRARY")).unwrap();
                #[cfg(unix)]
                let library = libloading::os::unix::Library::new(env!("FIXTURE_LIBRARY")).unwrap();
                ffi::set_library_handle(core::ptr::NonNull::new(library.into_raw() as *mut _))
                    .unwrap();
                ffi::set_load_policy(ffi::LoadPolicy::Explicit).unwrap();
            });
        }
        ffi::require(&[
            ffi::Symbol::WindowsHandlesHandleApiRaw,
            ffi::Symbol::WindowsHandlesHandleApiOptionalRaw,
            ffi::Symbol::WindowsHandlesHandleApiTake,
            ffi::Symbol::WindowsHandlesHandleApiOptionalTake,
            ffi::Symbol::WindowsHandlesHandleApiFromReady,
            ffi::Symbol::WindowsHandlesConsoleBuilderNew,
            ffi::Symbol::WindowsHandlesConsoleBuilderAddTtyPort,
            ffi::Symbol::WindowsHandlesConsoleBuilderDestroy,
            ffi::Symbol::WindowsHandlesInitLog,
            ffi::Symbol::WindowsHandlesNewConsole,
            ffi::Symbol::WindowsHandlesReadySourceReadyIsValid,
        ])
        .unwrap();
    }

    #[test]
    fn borrowed_owned_and_optional_handles_roundtrip() {
        init();
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        #[cfg(windows)]
        {
            assert_eq!(HandleApi::raw(file.as_handle()), file.as_raw_handle());
            assert!(HandleApi::optional_raw(None).is_null());
            assert_eq!(
                HandleApi::optional_raw(Some(file.as_handle())),
                file.as_raw_handle()
            );
            let owned: std::os::windows::io::OwnedHandle = file.try_clone().unwrap().into();
            let raw = owned.as_raw_handle();
            let returned = HandleApi::take(owned);
            assert_eq!(returned.as_raw_handle(), raw);
            assert!(std::fs::File::from(returned).metadata().is_ok());
            let owned: std::os::windows::io::OwnedHandle = file.try_clone().unwrap().into();
            let raw = owned.as_raw_handle();
            let returned = HandleApi::optional_take(Some(owned)).unwrap();
            assert_eq!(returned.as_raw_handle(), raw);
            assert!(std::fs::File::from(returned).metadata().is_ok());
            assert_eq!(
                new_console().add_tty_port(file.as_handle()).unwrap(),
                file.as_raw_handle()
            );
        }
        #[cfg(unix)]
        {
            assert_eq!(HandleApi::raw(file.as_fd()), file.as_raw_fd());
            assert_eq!(HandleApi::optional_raw(None), -1);
            assert_eq!(
                HandleApi::optional_raw(Some(file.as_fd())),
                file.as_raw_fd()
            );
            let owned: std::os::fd::OwnedFd = file.try_clone().unwrap().into();
            let raw = owned.as_raw_fd();
            let returned = HandleApi::take(owned);
            assert_eq!(returned.as_raw_fd(), raw);
            assert!(std::fs::File::from(returned).metadata().is_ok());
            let owned: std::os::fd::OwnedFd = file.try_clone().unwrap().into();
            let raw = owned.as_raw_fd();
            let returned = HandleApi::optional_take(Some(owned)).unwrap();
            assert_eq!(returned.as_raw_fd(), raw);
            assert!(std::fs::File::from(returned).metadata().is_ok());
            assert_eq!(
                new_console().add_tty_port(file.as_fd()).unwrap(),
                file.as_raw_fd()
            );
        }
        assert!(HandleApi::optional_take(None).is_none());
        assert!(!init_log(None));
    }

    #[test]
    fn callback_returns_a_handle_borrowed_from_its_owner() {
        init();
        struct ReadyFile(std::fs::File);
        impl ReadySource for ReadyFile {
            #[cfg(unix)]
            fn ready(&self) -> std::os::fd::BorrowedFd<'_> {
                self.0.as_fd()
            }
            #[cfg(windows)]
            fn ready(&self) -> std::os::windows::io::BorrowedHandle<'_> {
                self.0.as_handle()
            }
        }
        let callback = ReadyFile(std::fs::File::open(std::env::current_exe().unwrap()).unwrap());
        #[cfg(unix)]
        assert_eq!(HandleApi::from_ready(&callback), callback.0.as_raw_fd());
        #[cfg(windows)]
        assert_eq!(HandleApi::from_ready(&callback), callback.0.as_raw_handle());
        assert!(callback.0.metadata().is_ok());
    }
}

#[cfg(all(test, feature = "ffier-dynlookup", not(feature = "ffier-strong")))]
#[allow(dead_code)]
mod lazy_client {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));

    #[test]
    fn default_lazy_policy_loads_from_explicit_native_handle() {
        unsafe {
            #[cfg(windows)]
            let library = libloading::os::windows::Library::new(env!("FIXTURE_LIBRARY")).unwrap();
            #[cfg(unix)]
            let library = libloading::os::unix::Library::new(env!("FIXTURE_LIBRARY")).unwrap();
            ffi::set_library_handle(core::ptr::NonNull::new(library.into_raw() as *mut _)).unwrap();
        }
        assert!(!init_log(None));
        assert!(ffi::set_load_policy(ffi::LoadPolicy::Explicit).is_err());
        assert!(unsafe { ffi::set_library_handle(None) }.is_err());
    }
}

#[cfg(all(
    test,
    windows,
    feature = "ffier-dynlookup",
    not(feature = "ffier-strong")
))]
#[allow(dead_code)]
mod executable_client {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));

    #[test]
    fn default_source_does_not_search_loaded_dlls() {
        let _library =
            unsafe { libloading::os::windows::Library::new(env!("FIXTURE_LIBRARY")).unwrap() };
        assert!(!ffi::Symbol::WindowsHandlesInitLog.is_available());
        let error = ffi::require(&[ffi::Symbol::WindowsHandlesInitLog]).unwrap_err();
        assert!(std::error::Error::source(&error).is_some());
    }
}
