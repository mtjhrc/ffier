//! features: native via-cdylib
//! expected-error: cannot be shared between threads safely

#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use std::cell::Cell;

struct LocalSyncBackend(Cell<i32>);

impl api::SyncBackend for LocalSyncBackend {
    fn value(&self) -> i32 {
        self.0.get()
    }
}

fn main() {}
