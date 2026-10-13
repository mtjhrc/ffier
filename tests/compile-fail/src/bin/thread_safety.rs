#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn require_send<T: Send>() {}
fn require_sync<T: Sync>() {}
fn require_send_value<T: Send>(_: T) {}
fn require_sync_value<T: Sync>(_: T) {}

#[cfg(feature = "thread_not_send")]
struct LocalBackend(std::rc::Rc<i32>);

#[cfg(feature = "thread_not_send")]
impl api::SendBackend for LocalBackend {
    fn step(&mut self) -> i32 {
        *self.0
    }
}

#[cfg(feature = "thread_not_send_sync")]
impl api::ThreadBackend for LocalSyncBackend {
    fn value(&self) -> i32 {
        self.0.get()
    }
}

#[cfg(any(feature = "thread_not_sync", feature = "thread_not_send_sync"))]
struct LocalSyncBackend(std::cell::Cell<i32>);
#[cfg(feature = "thread_not_sync")]
impl api::SyncBackend for LocalSyncBackend {
    fn value(&self) -> i32 {
        self.0.get()
    }
}

#[cfg(any(feature = "thread_not_static", feature = "thread_worker_escape"))]
struct BorrowedBackend<'a>(&'a i32);
#[cfg(any(feature = "thread_not_static", feature = "thread_worker_escape"))]
impl api::SendBackend for BorrowedBackend<'_> {
    fn step(&mut self) -> i32 {
        *self.0
    }
}

#[cfg(feature = "thread_factory_wrong_guarantees")]
struct BadFactory;
#[cfg(feature = "thread_factory_wrong_guarantees")]
impl api::SendBackend for BadFactory {
    fn step(&mut self) -> i32 {
        0
    }

    fn __into_callback_handle<'callback>(
        self,
    ) -> ffier::CallbackHandle<'callback, api::__FfierCallbackTagSendBackend>
    where
        Self: Sized + 'callback,
    {
        unimplemented!()
    }
}

fn main() {
    #[cfg(feature = "thread_not_static")]
    {
        let local = 1;
        api::run_send_backend(BorrowedBackend(&local));
    }
    #[cfg(feature = "thread_worker_escape")]
    {
        let worker = {
            let local = 1;
            api::BackendWorker::new(BorrowedBackend(&local))
        };
        worker.run();
    }
    #[cfg(feature = "thread_handle_not_send")]
    require_send::<api::Gadget>();
    #[cfg(feature = "thread_handle_not_sync")]
    require_sync::<api::Gadget>();
    #[cfg(feature = "thread_send_handle_not_sync")]
    require_sync::<api::SendCounter>();
    #[cfg(feature = "thread_sync_handle_not_send")]
    require_send::<api::SyncCounter>();
    #[cfg(feature = "thread_vtable_not_send")]
    require_send::<api::VtableFruit<'static>>();
    #[cfg(feature = "thread_vtable_not_sync")]
    require_sync::<api::VtableFruit<'static>>();
    #[cfg(feature = "thread_send_vtable_not_sync")]
    require_sync::<api::VtableSendBackend<'static>>();
    #[cfg(feature = "thread_sync_vtable_not_send")]
    require_send::<api::VtableSyncBackend<'static>>();
    #[cfg(feature = "thread_callback_not_send")]
    {
        use api::Fruit;
        require_send_value(api::Apple::new(1).__into_callback_handle());
    }
    #[cfg(feature = "thread_callback_not_sync")]
    {
        use api::Fruit;
        require_sync_value(api::Apple::new(1).__into_callback_handle());
    }
    #[cfg(feature = "thread_send_callback_not_sync")]
    {
        use api::SendBackend;
        require_sync_value(api::SendCounter::new(1).__into_callback_handle());
    }
    #[cfg(feature = "thread_sync_callback_not_send")]
    {
        use api::SyncBackend;
        require_send_value(api::SyncCounter::new(1).__into_callback_handle());
    }
    #[cfg(feature = "thread_shared_send_not_send")]
    {
        use api::SendBackend;
        let backend = api::SendCounter::new(1);
        require_send_value(backend.__borrow_callback_handle());
    }
    #[cfg(feature = "thread_raw_vtable_not_send")]
    require_send::<ffier::VtableHandle>();
    #[cfg(feature = "thread_raw_vtable_not_sync")]
    require_sync::<ffier::VtableHandle>();
}
