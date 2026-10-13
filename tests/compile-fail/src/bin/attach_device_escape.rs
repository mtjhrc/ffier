#[cfg(feature = "native")]
use ffier_test_lib as api;
#[cfg(feature = "via-cdylib")]
use ffier_test_lib_via_cdylib as api;

fn main() {
    let device;
    {
        let label = String::from("attached");
        device = api::AttachItem::new(&label);
    }
    let _ = api::attach_device(device);
}
