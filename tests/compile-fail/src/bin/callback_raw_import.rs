//! features: via-cdylib
//! expected-error: unsafe and requires unsafe block

use ffier_test_lib_via_cdylib as api;

fn main() {
    let raw = api::Widget::new().__into_raw();
    let _forged = api::Widget::__from_raw(raw);
}
