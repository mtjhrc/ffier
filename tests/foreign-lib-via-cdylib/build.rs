fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let lib = ffier_test_codegen::load_schema(workspace_root, "ffier-fl.json")
        .expect("foreign lib schema");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    ffier_test_codegen::write_rust_client(&lib, std::path::Path::new(&out_dir), None)
        .expect("foreign client bindings");

    link_foreign_lib(workspace_root);
}

/// Link against the foreign lib cdylib. The cdylib itself is built by the
/// `build-foreign` recipe in tests/justfile before any test crate runs.
fn link_foreign_lib(workspace_root: &std::path::Path) {
    let target_dir = workspace_root.join("target").join("debug");
    println!("cargo:rustc-link-search=native={}", target_dir.display());
    let name = if cfg!(windows) {
        "ffier_test_foreign_lib.dll"
    } else {
        "ffier_test_foreign_lib"
    };
    println!("cargo:rustc-link-lib=dylib={name}");
}
