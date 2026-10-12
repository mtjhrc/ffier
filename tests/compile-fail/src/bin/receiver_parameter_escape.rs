#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn main() {
    let source = api::Widget::new();
    let mut view = api::View::create(&source);
    let widget = api::Widget::new();
    widget.replace_view(&mut view);
    drop(widget);
    let _ = view.source_count();
}
