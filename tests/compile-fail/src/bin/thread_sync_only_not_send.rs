//! features: via-cdylib
//! expected-error: cannot be sent between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use api::SyncBackend;

fn require_send_value<T: Send>(_: T) {}

fn main() {
    require_send_value(api::SyncCounter::new(1).__into_callback_handle());
}
