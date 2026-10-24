#![allow(dead_code)]

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

fn assert_closes_fd(operation: impl FnOnce(std::os::fd::OwnedFd) + std::panic::UnwindSafe) {
    use std::io::Read;
    let (owned, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    panic_message(|| operation(owned.into()));
    assert_eq!(
        peer.read(&mut [0u8; 1]).unwrap(),
        0,
        "owned descriptor leaked"
    );
}

pub fn assert_failed_calls_preserve_ownership() {
    use linking_client::{FdWorker, Symbol, Widget, require};
    require(&[
        Symbol::FixtureWidgetDestroy,
        Symbol::FixtureDestroyCount,
        Symbol::FixtureConsumeFdCallback,
    ])
    .unwrap();

    assert_closes_fd(linking_client::consume_fd);
    assert_closes_fd(Widget::consume_fd);

    let widget = || Widget::__from_raw(core::ptr::NonNull::<u8>::dangling().as_ptr().cast());
    let before = linking_client::destroy_count();
    panic_message(|| widget().consume());
    assert_eq!(linking_client::destroy_count(), before + 1);
    panic_message(|| linking_client::consume_widget(widget()));
    assert_eq!(linking_client::destroy_count(), before + 2);

    let concrete = widget();
    assert_closes_fd(|fd| FdWorker::consume_fd(&concrete, fd));
    struct DefaultCallback;
    impl FdWorker for DefaultCallback {}
    assert_closes_fd(|fd| FdWorker::consume_fd(&DefaultCallback, fd));

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
    assert_closes_fd(|fd| linking_client::consume_fd_callback(fd, Callback(drops.clone())));
    assert_eq!(drops.load(Ordering::Relaxed), 1);

    // A concrete handle implementation must not require callback allocation.
    use std::io::Read;
    let (owned, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    let before = linking_client::destroy_count();
    linking_client::consume_fd_callback(owned.into(), widget());
    assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
    assert_eq!(linking_client::destroy_count(), before + 1);
}
