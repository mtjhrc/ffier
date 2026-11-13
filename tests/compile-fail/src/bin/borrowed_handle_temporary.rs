//! expected-error: temporary value dropped while borrowed
//! features: via-cdylib weak-client

use ffier_test_lib_via_cdylib as api;

fn main() {
    let borrowed = api::Widget::new().gadget();
    let _ = borrowed.get();
}
