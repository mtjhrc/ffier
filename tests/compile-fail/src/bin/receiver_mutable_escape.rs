#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn main() {
    let mut widget = api::Widget::new();
    let view = widget.view_mut();
    widget.set_count(42);
    let _ = view.source_count();
}
