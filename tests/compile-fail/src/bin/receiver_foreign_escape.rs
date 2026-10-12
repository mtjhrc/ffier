#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn main() {
    let widget = api::Widget::new();
    let mut value = ffier_test_foreign_lib_via_cdylib::ForeignBorrowed::new("");
    widget.replace_foreign_borrowed(&mut value);
    drop(widget);
    let _ = value.value();
}
