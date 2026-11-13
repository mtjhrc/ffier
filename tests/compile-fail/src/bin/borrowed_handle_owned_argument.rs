//! expected-error: mismatched types
//! features: via-cdylib weak-client

use ffier_test_lib_via_cdylib as api;

fn main() {
    let owner = api::Widget::new();
    let borrowed = owner.gadget();
    let _ = api::gadget_value(borrowed);
}
