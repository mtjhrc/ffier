//! features: via-cdylib
//! expected-error: cannot be sent between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn require_send<T: Send>() {}

fn main() {
    // Gadget has no send/sync annotation, so the generated client handle
    // grants neither thread transfer nor sharing.
    require_send::<api::Gadget>();
}
