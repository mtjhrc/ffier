//! features: via-cdylib
//! expected-error: cannot be sent between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use api::Fruit;

fn require_send_value<T: Send>(_: T) {}

fn main() {
    // Fruit has no thread-safety supertraits, so its callback handles
    // grant neither transfer nor sharing.
    require_send_value(api::Apple::new(1).__into_callback_handle());
}
