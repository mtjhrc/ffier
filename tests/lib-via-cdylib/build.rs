#[cfg(unix)]
fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let target_dir = workspace_root.join("target").join("debug");
    println!("cargo:rustc-link-search=native={}", target_dir.display());
    println!("cargo:rustc-link-lib=dylib=ffier_test_lib");
    if std::env::var_os("CARGO_FEATURE_WEAK_TESTS").is_some() {
        let schema_path = workspace_root.join("target/ffier-ft.json");
        println!("cargo:rerun-if-changed={}", schema_path.display());
        let source = ffier_gen_rust_client::generate_from_file_with_options(
            schema_path.to_str().unwrap(),
            &ffier_gen_rust_client::Options { weak: true },
        )
        .unwrap();
        let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        std::fs::write(out_dir.join("generated_weak.rs"), source).unwrap();
    }
}

#[cfg(not(unix))]
fn main() {}
