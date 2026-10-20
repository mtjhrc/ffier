//! features: via-cdylib
//! expected-error: cannot be shared between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use api::SendBackend;

fn require_sync_value<T: Sync>(_: T) {}

fn main() {
    let mut backend = api::SendCounter::new(1);
    let _ = backend.step();
    require_sync_value(backend.__borrow_callback_handle_mut());
}
