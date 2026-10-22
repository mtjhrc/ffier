// Compile unchanged against native Rust and generated strong/weak bindings.
//
// Exercises thread-safety contracts across the three client surfaces:
//   - native: ffier-test-lib with the plain Rust traits and types
//   - strong: lib-via-cdylib with linked generated bindings
//   - weak:   the same bindings with lazy symbol loading
//
// Fixtures stay scalar (i32 counters) so the same source covers every mode
// without value structs or library-specific payloads.
//
// The harness including this file must define `fn init()` and expose the
// library surface as `api`. Every test calls `init()` first; the weak
// harness resolves the required dynamic symbols there.

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use api::SendBackend as _;
use api::SyncBackend as _;
use api::ThreadBackend as _;

// Capability assertions used by the client-only macro body below.
#[allow(dead_code)]
fn require_send<T: Send>(_: &T) {}
#[allow(dead_code)]
fn require_sync<T: Sync>(_: &T) {}

/// Send fixture: ownership may move, shared concurrent access may not.
struct SendCallback {
    value: Cell<i32>,
    drops: Arc<AtomicUsize>,
}

impl api::SendBackend for SendCallback {
    fn step(&mut self) -> i32 {
        let value = self.value.get();
        self.value.set(value + 1);
        value
    }
}

/// Send + Sync fixture: full thread transfer and sharing.
struct ThreadCallback {
    value: i32,
    drops: Arc<AtomicUsize>,
}

impl api::ThreadBackend for ThreadCallback {
    fn value(&self) -> i32 {
        self.value
    }
}

/// Sync-only fixture. Rc is a marker only, never a stored allocation.
/// SAFETY: shared access touches only an immutable i32 and an atomic drop
/// counter. The marker deliberately withholds Send to exercise the
/// independent Sync guarantee.
struct SyncCallback {
    value: i32,
    drops: Arc<AtomicUsize>,
    not_send: PhantomData<Rc<()>>,
}

unsafe impl Sync for SyncCallback {}

impl api::SyncBackend for SyncCallback {
    fn value(&self) -> i32 {
        self.value
    }
}

impl Drop for SendCallback {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl Drop for ThreadCallback {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl Drop for SyncCallback {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn sync_callback(value: i32, drops: &Arc<AtomicUsize>) -> SyncCallback {
    SyncCallback {
        value,
        drops: drops.clone(),
        not_send: PhantomData,
    }
}

fn send_callback(value: i32, drops: &Arc<AtomicUsize>) -> SendCallback {
    SendCallback {
        value: Cell::new(value),
        drops: drops.clone(),
    }
}

fn thread_callback(value: i32, drops: &Arc<AtomicUsize>) -> ThreadCallback {
    ThreadCallback {
        value,
        drops: drops.clone(),
    }
}

#[test]
fn opaque_handles_expose_independent_verified_guarantees() {
    init();
    fn send<T: Send>() {}
    fn sync<T: Sync>() {}
    send::<api::SendCounter>();
    sync::<api::SyncCounter>();
    send::<api::BackendWorker<'_>>();
    send::<api::Widget>();
    sync::<api::Widget>();
}

#[test]
fn send_only_handle_transfers_ownership_to_a_real_thread() {
    init();
    let counter = api::SendCounter::new(21);
    let counter = std::thread::spawn(move || {
        assert_eq!(counter.current_value(), 21);
        counter
    })
    .join()
    .expect("send-only handle transfer panicked");
    drop(counter);
}

#[test]
fn sync_only_handle_shares_across_scoped_threads() {
    init();
    let counter = api::SyncCounter::new(7);
    std::thread::scope(|scope| {
        scope.spawn(|| assert_eq!(counter.current_value(), 7));
        scope.spawn(|| assert_eq!(counter.current_value(), 7));
    });
    drop(counter);
}

#[test]
fn backend_worker_retains_owned_callback_and_transfers_to_a_thread() {
    init();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut worker = api::BackendWorker::new(send_callback(41, &drops));
    assert_eq!(worker.step(), 41);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let result = std::thread::spawn(move || worker.run())
        .join()
        .expect("worker thread panicked");
    assert_eq!(result, 42);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn backend_worker_accepts_borrowing_backends_within_the_scope() {
    init();
    struct BorrowingBackend<'a>(&'a mut Cell<i32>);
    impl api::SendBackend for BorrowingBackend<'_> {
        fn step(&mut self) -> i32 {
            self.0.get()
        }
    }
    let mut value = Cell::new(81);
    let worker = api::BackendWorker::new(BorrowingBackend(&mut value));
    assert_eq!(worker.run(), 81);
    assert_eq!(value.get(), 81);
}

#[test]
fn exported_worker_functions_exercise_every_capability_shape() {
    init();
    let drops = Arc::new(AtomicUsize::new(0));
    // Owned 'static worker: the callback crosses a real spawned thread.
    assert_eq!(api::run_send_backend(send_callback(51, &drops)), 51);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    // Combined Send + Sync: two scoped workers plus a spawned thread.
    assert_eq!(api::run_thread_backend(thread_callback(17, &drops)), 51);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn exported_borrowing_functions_keep_scoped_ownership() {
    init();
    let drops = Arc::new(AtomicUsize::new(0));
    // Mutable Send: the library lends the borrow to a scoped worker.
    let mut callback = send_callback(10, &drops);
    assert_eq!(api::borrow_send_backend(&mut callback), 10);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(callback);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    // Shared Sync: two scoped workers read the same callback.
    let callback = sync_callback(13, &drops);
    assert_eq!(api::share_sync_backend(&callback), 26);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    // Sync-only owned transfer: the callback moves into the library.
    assert_eq!(api::own_sync_backend(callback), 26);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn trait_implementations_follow_verified_supertraits() {
    init();
    // SendBackend requires Send: the fixture moves to a real thread.
    let drops = Arc::new(AtomicUsize::new(0));
    let callback = send_callback(5, &drops);
    let mut callback = std::thread::spawn(move || callback)
        .join()
        .expect("send fixture transfer panicked");
    assert_eq!(callback.step(), 5);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    // SyncBackend requires Sync: shared scoped access on the !Send fixture.
    let callback = sync_callback(6, &drops);
    std::thread::scope(|scope| {
        scope.spawn(|| assert_eq!(callback.value(), 6));
        scope.spawn(|| assert_eq!(callback.value(), 6));
    });
    drop(callback);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(send_callback(5, &drops));
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn thread_backend_default_method_reaches_the_callback() {
    init();
    let drops = Arc::new(AtomicUsize::new(0));
    let callback = thread_callback(20, &drops);
    // twice() is a default method on ThreadBackend; the library dispatches
    // it through the same verified Send + Sync contract.
    std::thread::scope(|scope| {
        scope.spawn(|| assert_eq!(callback.twice(), 40));
    });
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(callback);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

// Client-only CallbackHandle tests. The macro body references the fixtures
// from this file and expands at the harness site, so the tests live here
// while the harness decides whether to run them.
#[allow(unused_macros)]
macro_rules! ffier_thread_client_body {
    () => {
        mod callback_handle_capabilities {
            use super::*;

            #[test]
            fn owned_send_callback_handle_transfers_and_drops_exactly_once() {
                init();
                let drops = Arc::new(AtomicUsize::new(0));
                let handle = send_callback(2, &drops).__into_callback_handle();
                require_send(&handle);
                std::thread::spawn(move || drop(handle))
                    .join()
                    .expect("owned handle transfer panicked");
                assert_eq!(drops.load(Ordering::SeqCst), 1);
            }

            #[test]
            fn mutable_borrow_handle_transfers_within_the_scope() {
                init();
                let drops = Arc::new(AtomicUsize::new(0));
                let mut callback = send_callback(3, &drops);
                let handle = callback.__borrow_callback_handle_mut();
                require_send(&handle);
                std::thread::scope(|scope| {
                    scope.spawn(move || drop(handle));
                });
                assert_eq!(drops.load(Ordering::SeqCst), 0);
                drop(callback);
                assert_eq!(drops.load(Ordering::SeqCst), 1);
            }

            #[test]
            fn shared_sync_callback_handle_transfers_and_shares() {
                init();
                let drops = Arc::new(AtomicUsize::new(0));
                let callback = sync_callback(4, &drops);
                let handle = callback.__borrow_callback_handle();
                require_send(&handle);
                require_sync(&handle);
                std::thread::scope(|scope| {
                    scope.spawn(move || drop(handle));
                });
                let handle = callback.__into_callback_handle();
                require_sync(&handle);
                drop(handle);
                assert_eq!(drops.load(Ordering::SeqCst), 1);
            }

            #[test]
            fn combined_callback_handle_carries_both_capabilities() {
                init();
                let drops = Arc::new(AtomicUsize::new(0));
                let handle = thread_callback(5, &drops).__into_callback_handle();
                require_send(&handle);
                require_sync(&handle);
                std::thread::spawn(move || drop(handle))
                    .join()
                    .expect("combined handle transfer panicked");
                assert_eq!(drops.load(Ordering::SeqCst), 1);
            }

            #[test]
            fn send_only_shared_borrow_handle_stays_on_the_creating_thread() {
                init();
                // A Send-only backend's shared borrow handle is neither Send nor
                // Sync, so it must not cross threads. The compile-fail fixtures
                // prove rejection; here dropping it on the owning thread still
                // destroys the borrowed shell exactly once without touching the
                // callback.
                let drops = Arc::new(AtomicUsize::new(0));
                let callback = send_callback(6, &drops);
                let handle = callback.__borrow_callback_handle();
                drop(handle);
                assert_eq!(drops.load(Ordering::SeqCst), 0);
                drop(callback);
                assert_eq!(drops.load(Ordering::SeqCst), 1);
            }

            #[test]
            fn factory_return_types_track_ownership_and_capabilities() {
                init();
                // The factory signatures themselves carry the contract: the owned
                // factory of a Send-only trait yields an owned Send handle, and
                // the shared factory of a Sync-only trait yields a shared handle
                // with the Sync capability. Typing these bindings pins the shape
                // the generator must keep emitting.
                let drops = Arc::new(AtomicUsize::new(0));
                let owned: ffier::CallbackHandle<
                    '_,
                    api::__FfierCallbackTagSendBackend,
                    ffier::OwnedCallback,
                    true,
                    false,
                > = send_callback(7, &drops).__into_callback_handle();
                drop(owned);
                assert_eq!(drops.load(Ordering::SeqCst), 1);
                let callback = sync_callback(8, &drops);
                let shared: ffier::CallbackHandle<
                    '_,
                    api::__FfierCallbackTagSyncBackend,
                    ffier::BorrowedCallback<false>,
                    false,
                    true,
                > = callback.__borrow_callback_handle();
                drop(shared);
                assert_eq!(drops.load(Ordering::SeqCst), 1);
                drop(callback);
                assert_eq!(drops.load(Ordering::SeqCst), 2);
            }
        }
    };
}

// The native harness never invokes the body: plain Rust traits carry no
// CallbackHandle factories. Generated client harnesses (strong and weak
// bindings) invoke `ffier_thread_client_body!();` after including this
// file, which expands the tests below against their generated traits.
