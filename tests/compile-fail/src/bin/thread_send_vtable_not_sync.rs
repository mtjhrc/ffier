//! features: via-cdylib
//! expected-error: cannot be shared between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn require_sync<T: Sync>() {}

fn main() {
    // SendBackend is Send-only: its vtable wrapper moves but never shares.
    require_sync::<api::VtableSendBackend<'static>>();
}
