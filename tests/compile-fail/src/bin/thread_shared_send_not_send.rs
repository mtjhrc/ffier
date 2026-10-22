//! features: via-cdylib
//! expected-error: cannot be sent between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use api::SendBackend;

fn require_send_value<T: Send>(_: T) {}

fn main() {
    let backend = api::SendCounter::new(1);
    require_send_value(backend.__borrow_callback_handle());
}
