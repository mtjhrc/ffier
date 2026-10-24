#![allow(dead_code)]

#[cfg(unix)]
pub fn open_library(path: &str, global: bool) -> core::ptr::NonNull<core::ffi::c_void> {
    let library = unsafe {
        libloading::os::unix::Library::open(
            Some(path),
            libloading::os::unix::RTLD_NOW
                | if global {
                    libloading::os::unix::RTLD_GLOBAL
                } else {
                    libloading::os::unix::RTLD_LOCAL
                },
        )
        .unwrap()
    };
    // Keep the provider loaded for the lifetime of the test process.
    core::ptr::NonNull::new(library.into_raw()).unwrap()
}

#[cfg(windows)]
pub fn open_library(path: &str, _global: bool) -> core::ptr::NonNull<core::ffi::c_void> {
    let library = unsafe { libloading::os::windows::Library::new(path) }.unwrap();
    core::ptr::NonNull::new(library.into_raw() as *mut core::ffi::c_void).unwrap()
}

pub fn panic_message<T>(operation: impl FnOnce() -> T + std::panic::UnwindSafe) -> String {
    let panic = std::panic::catch_unwind(operation)
        .err()
        .expect("expected a panic");
    if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        panic.downcast_ref::<&str>().unwrap().to_string()
    }
}

pub fn assert_failed_callback_allocation_drops_value() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Callback(Arc<AtomicUsize>);
    impl linking_client::Worker for Callback {}
    impl Drop for Callback {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let callback = Callback(drops.clone());
    let message = panic_message(|| linking_client::Worker::__into_raw_handle(callback));
    assert!(message.contains("fixture_vtable_handle_new"));
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[cfg(unix)]
type OwnedResource = std::os::fd::OwnedFd;
#[cfg(windows)]
type OwnedResource = std::os::windows::io::OwnedHandle;

#[cfg(unix)]
fn resource_pipe() -> (OwnedResource, impl FnOnce()) {
    use std::io::Read;
    let (owned, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    (owned.into(), move || {
        assert_eq!(
            peer.read(&mut [0u8; 1]).unwrap(),
            0,
            "owned descriptor leaked"
        )
    })
}

#[cfg(windows)]
fn resource_pipe() -> (OwnedResource, impl FnOnce()) {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreatePipe(
            read: *mut RawHandle,
            write: *mut RawHandle,
            attributes: *mut core::ffi::c_void,
            size: u32,
        ) -> i32;
        fn PeekNamedPipe(
            pipe: RawHandle,
            buffer: *mut core::ffi::c_void,
            size: u32,
            read: *mut u32,
            available: *mut u32,
            left: *mut u32,
        ) -> i32;
    }
    let mut read = core::ptr::null_mut();
    let mut write = core::ptr::null_mut();
    assert_ne!(
        unsafe { CreatePipe(&mut read, &mut write, core::ptr::null_mut(), 0) },
        0
    );
    let read = unsafe { OwnedHandle::from_raw_handle(read) };
    let write = unsafe { OwnedHandle::from_raw_handle(write) };
    (write, move || {
        let result = unsafe {
            PeekNamedPipe(
                read.as_raw_handle(),
                core::ptr::null_mut(),
                0,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        assert_eq!(result, 0, "owned handle leaked");
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(109)); // ERROR_BROKEN_PIPE
    })
}

fn assert_closes_resource(operation: impl FnOnce(OwnedResource) + std::panic::UnwindSafe) {
    let (owned, assert_closed) = resource_pipe();
    panic_message(|| operation(owned));
    assert_closed();
}

pub fn assert_failed_calls_preserve_ownership() {
    use linking_client::{FdWorker, Symbol, Widget, require};
    require(&[
        Symbol::FixtureWidgetDestroy,
        Symbol::FixtureDestroyCount,
        Symbol::FixtureConsumeFdCallback,
    ])
    .unwrap();

    assert_closes_resource(linking_client::consume_fd);
    assert_closes_resource(Widget::consume_fd);

    let widget = || Widget::__from_raw(core::ptr::NonNull::<u8>::dangling().as_ptr().cast());
    let before = linking_client::destroy_count();
    panic_message(|| widget().consume());
    assert_eq!(linking_client::destroy_count(), before + 1);
    panic_message(|| linking_client::consume_widget(widget()));
    assert_eq!(linking_client::destroy_count(), before + 2);

    let concrete = widget();
    assert_closes_resource(|fd| FdWorker::consume_fd(&concrete, fd));
    struct DefaultCallback;
    impl FdWorker for DefaultCallback {}
    assert_closes_resource(|fd| FdWorker::consume_fd(&DefaultCallback, fd));

    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Callback(Arc<AtomicUsize>);
    impl linking_client::Worker for Callback {}
    impl Drop for Callback {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    assert_closes_resource(|fd| linking_client::consume_fd_callback(fd, Callback(drops.clone())));
    assert_eq!(drops.load(Ordering::Relaxed), 1);

    // A concrete handle implementation must not require callback allocation.
    let (owned, assert_closed) = resource_pipe();
    let before = linking_client::destroy_count();
    linking_client::consume_fd_callback(owned, widget());
    assert_closed();
    assert_eq!(linking_client::destroy_count(), before + 1);
}
