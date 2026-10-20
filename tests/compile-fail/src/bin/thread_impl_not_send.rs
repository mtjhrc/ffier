//! features: native via-cdylib
//! expected-error: cannot be sent between threads safely

#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use std::rc::Rc;

struct LocalBackend(Rc<i32>);

impl api::SendBackend for LocalBackend {
    fn step(&mut self) -> i32 {
        *self.0
    }
}

fn main() {}
