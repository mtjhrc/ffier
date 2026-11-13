//! expected-error: `gadget` does not live long enough
//! features: via-cdylib weak-client

use ffier_test_lib_via_cdylib as api;

fn main() {
    let mut borrowed;
    {
        let owner = api::Widget::new();
        let mut gadget = owner.create_gadget();
        borrowed = api::gadget_identity_mut(&mut gadget);
    }
    borrowed.set(10);
}
