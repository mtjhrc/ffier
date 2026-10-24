use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=schema.json");
    println!("cargo:rerun-if-changed=provider.c");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let schema =
        ffier_schema::Library::from_json(&fs::read_to_string("schema.json").unwrap()).unwrap();
    let source = ffier_gen_rust_client::generate(&schema);
    // Identical input must yield identical output, including symbol metadata.
    assert_eq!(source, ffier_gen_rust_client::generate(&schema));
    fs::write(out_dir.join("generated.rs"), source).unwrap();

    let macos = env::var("CARGO_CFG_TARGET_OS").unwrap() == "macos";
    let library = out_dir.join(if macos {
        "liblinking_fixture.dylib"
    } else {
        "liblinking_fixture.so"
    });
    assert!(
        Command::new("cc")
            .args([
                if macos { "-dynamiclib" } else { "-shared" },
                "-fPIC",
                "provider.c",
                "-o"
            ])
            .arg(&library)
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-env=FIXTURE_LIBRARY={}", library.display());
    let other_library = out_dir.join(if macos {
        "liblinking_other.dylib"
    } else {
        "liblinking_other.so"
    });
    assert!(
        Command::new("cc")
            .args([
                if macos { "-dynamiclib" } else { "-shared" },
                "-fPIC",
                "-DOTHER_LIBRARY",
                "provider.c",
                "-o",
            ])
            .arg(&other_library)
            .status()
            .unwrap()
            .success()
    );
    println!(
        "cargo:rustc-env=FIXTURE_OTHER_LIBRARY={}",
        other_library.display()
    );
    if env::var_os("CARGO_FEATURE_STRONG").is_some() {
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=dylib=linking_fixture");
        // Use the built provider for this client's test executables.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", out_dir.display());
    }
}
