//! features: native via-cdylib
//! expected-error: does not live long enough

#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

struct BorrowedBackend<'a>(&'a i32);

impl api::SendBackend for BorrowedBackend<'_> {
    fn step(&mut self) -> i32 {
        *self.0
    }
}

fn main() {
    let local = 1;
    let _ = api::run_send_backend(BorrowedBackend(&local));
}
