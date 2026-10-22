//! features: via-cdylib
//! expected-error: cannot be sent between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn require_send<T: Send>() {}

fn main() {
    // SyncBackend is Sync-only: its vtable wrapper shares but never moves.
    require_send::<api::VtableSyncBackend<'static>>();
}
