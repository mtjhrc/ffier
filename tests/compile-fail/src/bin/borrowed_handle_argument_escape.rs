//! expected-error: `gadget` does not live long enough
//! features: via-cdylib weak-client

use ffier_test_lib_via_cdylib as api;

fn main() {
    let borrowed;
    {
        let owner = api::Widget::new();
        let gadget = owner.create_gadget();
        borrowed = api::gadget_identity(&gadget);
    }
    let _ = borrowed.get();
}
