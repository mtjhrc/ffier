//! features: via-cdylib
//! expected-error: cannot be shared between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use api::Fruit;

fn require_sync_value<T: Sync>(_: T) {}

fn main() {
    // Fruit has no thread-safety supertraits, so its callback handles
    // grant neither transfer nor sharing.
    require_sync_value(api::Apple::new(1).__into_callback_handle());
}
