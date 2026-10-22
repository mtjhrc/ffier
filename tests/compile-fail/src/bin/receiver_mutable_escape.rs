//! expected-error: cannot borrow `widget` as mutable more than once at a time
//! features: receiver-escapes

use ffier_test_lib_via_cdylib as api;

fn main() {
    let mut widget = api::Widget::new();
    let view = widget.view_mut();
    widget.set_count(42);
    let _ = view.source_count();
}
