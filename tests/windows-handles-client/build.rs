fn main() {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let _ = std::any::TypeId::of::<ffier_test_windows_handles::HandleApi>();
    let schema = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../target/ffier-windows-handles.json");
    println!("cargo:rerun-if-changed={}", schema.display());
    let generated = ffier_gen_rust_client::generate_from_file(schema.to_str().unwrap()).unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("generated.rs");
    fs::write(output, generated).unwrap();
}
