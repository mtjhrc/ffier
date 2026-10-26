fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let lib =
        ffier_test_codegen::load_schema(workspace_root, "ffier-ft.json").expect("test lib schema");
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());

    ffier_test_codegen::write_rust_client(&lib, &out_dir, None).expect("client bindings");

    #[cfg(unix)]
    link_test_lib(workspace_root);
}

/// Locate the test lib cdylib, linking it only in strong mode. It is built by the
/// `build` recipe in tests/justfile before any test crate runs.
#[cfg(unix)]
fn link_test_lib(workspace_root: &std::path::Path) {
    let target_dir = workspace_root.join("target").join("debug");
    println!("cargo:rustc-link-search=native={}", target_dir.display());
    #[cfg(feature = "ffier-strong")]
    println!("cargo:rustc-link-lib=dylib=ffier_test_lib");
}
