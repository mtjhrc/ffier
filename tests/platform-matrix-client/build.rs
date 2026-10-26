fn main() {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let _ = std::any::TypeId::of::<ffier_test_platform_matrix::HandleApi>();
    let schema = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../target/ffier-platform-matrix.json");
    let filename = match env::var("CARGO_CFG_TARGET_OS").unwrap().as_str() {
        "windows" => "ffier_test_platform_matrix.dll",
        "macos" => "libffier_test_platform_matrix.dylib",
        _ => "libffier_test_platform_matrix.so",
    };
    println!(
        "cargo:rustc-env=HANDLE_LIBRARY={}",
        schema
            .parent()
            .unwrap()
            .join("debug")
            .join(filename)
            .display()
    );
    println!("cargo:rerun-if-changed={}", schema.display());
    let generated = ffier_gen_rust_client::generate_from_file(schema.to_str().unwrap()).unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("generated.rs");
    fs::write(output, generated).unwrap();
}
