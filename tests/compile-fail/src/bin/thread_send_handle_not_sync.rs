//! features: via-cdylib
//! expected-error: cannot be shared between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn require_sync<T: Sync>() {}

fn main() {
    // SendCounter is verified Send-only: the client handle shares nothing.
    require_sync::<api::SendCounter>();
}
