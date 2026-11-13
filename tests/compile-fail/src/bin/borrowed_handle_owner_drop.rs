//! expected-error: cannot move out of `owner` because it is borrowed
//! features: via-cdylib weak-client

use ffier_test_lib_via_cdylib as api;

fn main() {
    let owner = api::Widget::new();
    let borrowed = owner.gadget();
    drop(owner);
    let _ = borrowed.get();
}
