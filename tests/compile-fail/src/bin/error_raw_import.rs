use ffier_test_lib_via_cdylib as api;

fn main() {
    let _error = api::TestError::from_ffi(0, core::ptr::null_mut());
}
