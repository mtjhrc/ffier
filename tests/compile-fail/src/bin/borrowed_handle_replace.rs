//! expected-error: cannot borrow data in dereference of
//! features: via-cdylib weak-client

use ffier_test_lib_via_cdylib as api;

fn main() {
    let mut owner = api::Widget::new();
    let other = api::Widget::new();
    let mut borrowed = owner.gadget_mut();
    let escaped = std::mem::replace(&mut *borrowed, other.create_gadget());
    drop(owner);
    let _ = escaped.get();
}
