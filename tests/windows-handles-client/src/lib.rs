#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

// Link the provider's Rust archive so strong extern calls resolve in this test.
use ffier_test_windows_handles as _;

#[cfg(all(test, feature = "strong"))]
mod tests {
    use super::*;

    #[test]
    fn platform_signatures_and_callback_slots_work() {
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::fd::{AsFd, AsRawFd, OwnedFd};
            assert_eq!(HandleApi::raw(file.as_fd()), file.as_raw_fd());
            assert_eq!(HandleApi::optional_raw(None), -1);
            let owned: OwnedFd = file.try_clone().unwrap().into();
            let raw = owned.as_raw_fd();
            assert_eq!(HandleApi::take(owned).as_raw_fd(), raw);
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::{AsHandle, AsRawHandle, OwnedHandle};
            assert_eq!(HandleApi::raw(file.as_handle()), file.as_raw_handle());
            assert!(HandleApi::optional_raw(None).is_null());
            let owned: OwnedHandle = file.try_clone().unwrap().into();
            let raw = owned.as_raw_handle();
            assert_eq!(HandleApi::take(owned).as_raw_handle(), raw);
        }
        assert!(HandleApi::optional_take(None).is_none());
        struct Callback;
        impl HandleConsumer for Callback {}
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
