// Thread-safe callbacks and independently verified opaque handle guarantees.
//
// Fixture API for tests/thread_client.rs: three scalar traits (Send-only,
// Sync-only, Send+Sync), counters with deliberately asymmetric native
// capabilities, an owning worker, and exported free functions exercising
// each ownership shape across real threads.

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

use crate::Widget;

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Send-only backend: ownership may move, shared concurrent access may not.
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait SendBackend: Send {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn step(&mut self) -> i32;
}

/// Sync-only backend: shared concurrent reads without ownership transfer.
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait SyncBackend: Sync {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn value(&self) -> i32;
}

/// Combined backend: full thread transfer and sharing, plus a default method.
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait ThreadBackend: Send + Sync {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn value(&self) -> i32;

    #[cfg_attr(feature = "ffi", ffier(index = 1))]
    fn twice(&self) -> i32 {
        self.value() * 2
    }
}

// ---------------------------------------------------------------------------
// Native fixtures with asymmetric capabilities
// ---------------------------------------------------------------------------

/// Send but not Sync: ownership may move, shared concurrent access may not.
pub struct SendCounter(Cell<i32>);

#[cfg_attr(feature = "ffi", ffier::export(send))]
impl SendCounter {
    pub fn new(value: i32) -> Self {
        Self(Cell::new(value))
    }

    pub fn current_value(&self) -> i32 {
        self.0.get()
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl SendBackend for SendCounter {
    fn step(&mut self) -> i32 {
        let value = self.0.get();
        self.0.set(value + 1);
        value
    }
}

/// Sync but not Send. Rc is a marker only, never a stored allocation.
pub struct SyncCounter {
    value: i32,
    not_send: PhantomData<Rc<()>>,
}

// SAFETY: only an immutable i32 is stored; shared access has no thread
// affinity. The marker deliberately withholds Send to exercise the
// independent Sync guarantee.
unsafe impl Sync for SyncCounter {}

#[cfg_attr(feature = "ffi", ffier::export(sync))]
impl SyncCounter {
    pub fn new(value: i32) -> Self {
        Self {
            value,
            not_send: PhantomData,
        }
    }

    pub fn current_value(&self) -> i32 {
        self.value
    }
}

#[cfg_attr(feature = "ffi", ffier::export)]
impl SyncBackend for SyncCounter {
    fn value(&self) -> i32 {
        self.value
    }
}

/// Widget carries both capabilities already, so it backs the combined trait.
#[cfg_attr(feature = "ffi", ffier::export)]
impl ThreadBackend for Widget {
    fn value(&self) -> i32 {
        self.get_count()
    }
}

// ---------------------------------------------------------------------------
// Owning worker
// ---------------------------------------------------------------------------

/// Retains a Send callback, then transfers ownership to a worker thread.
pub struct BackendWorker<'a> {
    backend: Box<dyn SendBackend + 'a>,
}

#[cfg_attr(feature = "ffi", ffier::export(send, invariant('a)))]
impl<'a> BackendWorker<'a> {
    pub fn new(
        #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] backend: impl SendBackend + 'a,
    ) -> Self {
        Self {
            backend: Box::new(backend),
        }
    }

    pub fn step(&mut self) -> i32 {
        self.backend.step()
    }

    pub fn run(mut self) -> i32 {
        std::thread::scope(|scope| {
            scope
                .spawn(move || self.backend.step())
                .join()
                .expect("backend worker panicked")
        })
    }
}

// ---------------------------------------------------------------------------
// Exported free functions — one per ownership shape
// ---------------------------------------------------------------------------

/// Owned 'static Send callback crossing a real spawned thread.
#[cfg_attr(feature = "ffi", ffier::export)]
// Exercise preservation of an explicit bound already implied by the trait.
#[allow(clippy::implied_bounds_in_impls)]
pub fn run_send_backend(
    #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] mut backend: impl SendBackend
    + Send
    + 'static,
) -> i32 {
    std::thread::spawn(move || backend.step())
        .join()
        .expect("backend worker panicked")
}

/// Mutable Send lend: one scoped worker step, callback stays with the caller.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn borrow_send_backend(
    #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] backend: &mut impl SendBackend,
) -> i32 {
    std::thread::scope(|scope| {
        scope
            .spawn(move || backend.step())
            .join()
            .expect("backend worker panicked")
    })
}

/// Shared Sync read: two scoped workers sum the same callback.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn share_sync_backend(
    #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] backend: &impl SyncBackend,
) -> i32 {
    std::thread::scope(|scope| {
        let first = scope.spawn(|| backend.value());
        let second = scope.spawn(|| backend.value());
        first.join().expect("backend worker panicked")
            + second.join().expect("backend worker panicked")
    })
}

/// Sync-only owned transfer: the callback moves into the library, shared read.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn own_sync_backend(
    #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] backend: impl SyncBackend,
) -> i32 {
    share_sync_backend(&backend)
}

/// Combined Send + Sync: a spawned thread that runs scoped workers on a copy.
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn run_thread_backend(
    #[cfg_attr(feature = "ffi", ffier(dispatch = vtable))] backend: impl ThreadBackend + 'static,
) -> i32 {
    std::thread::spawn(move || {
        std::thread::scope(|scope| {
            let first = scope.spawn(|| backend.value());
            let second = scope.spawn(|| backend.twice());
            first.join().expect("backend worker panicked")
                + second.join().expect("backend worker panicked")
        })
    })
    .join()
    .expect("backend worker panicked")
}
