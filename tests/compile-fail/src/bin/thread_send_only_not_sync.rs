//! features: via-cdylib
//! expected-error: cannot be shared between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use api::SendBackend;

fn require_sync_value<T: Sync>(_: T) {}

fn main() {
    require_sync_value(api::SendCounter::new(1).__into_callback_handle());
}
