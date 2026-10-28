// Shared by generated-client test harnesses. Each generated namespace has its
// own setter and caches, so initialize each with the corresponding DLL handle.
#[cfg(all(windows, not(feature = "ffier-strong")))]
fn load_test_dll(name: &str) -> core::ptr::NonNull<core::ffi::c_void> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
        .join(format!("{name}.dll"));
    let library = unsafe { libloading::os::windows::Library::new(path) }.unwrap();
    // Generated clients borrow the HMODULE for the lifetime of the process.
    core::ptr::NonNull::new(library.into_raw() as *mut core::ffi::c_void).unwrap()
}
