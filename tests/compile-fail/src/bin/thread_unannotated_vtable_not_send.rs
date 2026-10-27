//! features: via-cdylib
//! expected-error: cannot be sent between threads safely

#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn require_send<T: Send>() {}

fn main() {
    // Fruit has no thread-safety supertraits, so its vtable wrapper
    // grants neither transfer nor sharing.
    require_send::<api::VtableFruit<'static>>();
}
