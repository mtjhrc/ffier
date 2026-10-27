//! features: via-cdylib
//! expected-error: cannot be shared between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn require_sync<T: Sync>() {}

fn main() {
    // Gadget has no send/sync annotation, so the generated client handle
    // grants neither thread transfer nor sharing.
    require_sync::<api::Gadget>();
}
