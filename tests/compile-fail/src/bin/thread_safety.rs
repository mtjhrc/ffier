#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn require_send<T: Send>() {}
fn require_sync<T: Sync>() {}
fn main() {
    #[cfg(feature = "thread_handle_not_send")]
    require_send::<api::Gadget>();
    #[cfg(feature = "thread_handle_not_sync")]
    require_sync::<api::Gadget>();
    #[cfg(feature = "thread_send_handle_not_sync")]
    require_sync::<api::SendCounter>();
    #[cfg(feature = "thread_sync_handle_not_send")]
    require_send::<api::SyncCounter>();
}
