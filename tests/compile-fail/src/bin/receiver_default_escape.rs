//! expected-error: cannot move out of `widget` because it is borrowed
//! features: receiver-escapes

use ffier_test_lib_via_cdylib as api;

use api::ReceiverDefaults as _;

fn main() {
    let widget = api::Widget::with_name("borrowed label");
    let label = widget.choose("");
    drop(widget);
    println!("{label}");
}
