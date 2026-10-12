#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn main() {
    let widget = api::Widget::new();
    let view = widget.view();
    drop(widget);
    let _ = view.source_count();
}
