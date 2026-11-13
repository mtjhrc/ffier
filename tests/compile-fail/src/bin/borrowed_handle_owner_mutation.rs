//! expected-error: cannot borrow `owner` as mutable because it is also borrowed as immutable
//! features: via-cdylib weak-client

use ffier_test_lib_via_cdylib as api;

fn main() {
    let mut owner = api::Widget::new();
    let borrowed = owner.gadget();
    owner.set_count(1);
    let _ = borrowed.get();
}
