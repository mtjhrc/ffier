//! features: via-cdylib
//! expected-error: cannot be sent between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn require_send<T: Send>() {}

fn main() {
    // SyncCounter is verified Sync-only: the client handle never moves.
    require_send::<api::SyncCounter>();
}
