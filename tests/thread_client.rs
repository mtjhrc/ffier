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
    drop(callback);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}
