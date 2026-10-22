//! features: via-cdylib
//! expected-error: incompatible type for trait

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use api::SendBackend;

struct BadFactory;

impl SendBackend for BadFactory {
    fn step(&mut self) -> i32 {
        0
    }

    fn __into_callback_handle<'callback>(
        self,
    ) -> ffier::CallbackHandle<'callback, api::__FfierCallbackTagSendBackend>
    where
        Self: Sized + 'callback,
    {
        unimplemented!()
    }
}

fn main() {}
