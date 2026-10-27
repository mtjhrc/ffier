//! expected-error: cannot move out of `widget` because it is borrowed
//! features: receiver-escapes

use ffier_test_lib_via_cdylib as api;

fn main() {
    let widget = api::Widget::new();
    let view = widget.view();
    drop(widget);
    let _ = view.source_count();
}
