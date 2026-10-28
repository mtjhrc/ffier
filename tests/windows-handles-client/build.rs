fn main() {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let _ = std::any::TypeId::of::<ffier_test_windows_handles::HandleApi>();
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    println!("cargo:rerun-if-env-changed=FFIER_WINDOWS_HANDLES_SCHEMA");
    let schema = env::var_os("FFIER_WINDOWS_HANDLES_SCHEMA")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/ffier-windows-handles.json"));
    println!("cargo:rerun-if-changed={}", schema.display());
    let generated = ffier_gen_rust_client::generate_from_file(schema.to_str().unwrap()).unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("generated.rs");
    fs::write(output, generated).unwrap();

    let target = root.join("target/debug");
    let windows = env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows";
    let macos = env::var("CARGO_CFG_TARGET_OS").unwrap() == "macos";
    let name = if windows {
        "ffier_test_windows_handles.dll"
    } else if macos {
        "libffier_test_windows_handles.dylib"
    } else {
        "libffier_test_windows_handles.so"
    };
    println!(
        "cargo:rustc-env=FIXTURE_LIBRARY={}",
        target.join(name).display()
    );
    if env::var_os("CARGO_FEATURE_FFIER_STRONG").is_some() {
        println!("cargo:rustc-link-search=native={}", target.display());
        println!(
            "cargo:rustc-link-lib=dylib={}",
            if windows {
                "ffier_test_windows_handles.dll"
            } else {
                "ffier_test_windows_handles"
            }
        );
    }
}
