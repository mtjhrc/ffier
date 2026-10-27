//! expected-error: cannot be shared between threads safely
//! features: native via-cdylib weak-client

#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(any(feature = "via-cdylib", feature = "weak-client"))]
use ffier_test_lib_via_cdylib as api;

// MixedVariance is exported send-only: the withheld Sync guarantee must not
// be available in any client mode.
fn require_sync() {
    fn require<T: Sync>() {}
    require::<api::MixedVariance<'_, '_, '_, '_>>();
}

fn main() {}
