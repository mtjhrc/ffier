#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

use api::ReceiverDefaults as _;

fn main() {
    let widget = api::Widget::with_name("borrowed label");
    let label = widget.choose("");
    drop(widget);
    println!("{label}");
}
