//! expected-error: cannot borrow `owner` as mutable more than once at a time
//! features: via-cdylib weak-client

use ffier_test_lib_via_cdylib as api;

fn main() {
    let mut owner = api::Widget::new();
    let first = owner.gadget_mut();
    let second = owner.gadget_mut();
    let _ = first.get() + second.get();
}
