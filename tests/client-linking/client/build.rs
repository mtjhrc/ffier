use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=schema.json");
    println!("cargo:rerun-if-changed=provider.c");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let windows = env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows";
    let mut json = fs::read_to_string("schema.json").unwrap();
    if windows {
        json = json
            .replace("OwnedFd", "OwnedHandle")
            .replace("RawFd", "RawHandle")
            .replace("owned_fd", "owned_handle")
            .replace("raw_fd", "raw_handle");
    }
    let mut schema = ffier_schema::Library::from_json(&json).unwrap();
    if windows {
        schema.type_registry.get_mut("RawHandle").unwrap().kind =
            ffier_schema::TypeKind::Primitive {
                c_type: "void*".into(),
            };
    }
    let source = ffier_gen_rust_client::generate(&schema);
    // Identical input must yield identical output, including symbol metadata.
    assert_eq!(source, ffier_gen_rust_client::generate(&schema));
    fs::write(out_dir.join("generated.rs"), source).unwrap();

    let macos = env::var("CARGO_CFG_TARGET_OS").unwrap() == "macos";
    let filename = |name: &str| {
        if windows {
            format!("{name}.dll")
        } else if macos {
            format!("lib{name}.dylib")
        } else {
            format!("lib{name}.so")
        }
    };
    let library = out_dir.join(filename("linking_fixture"));
    let other_library = out_dir.join(filename("linking_other"));
    for (path, other) in [(&library, false), (&other_library, true)] {
        let status = if windows {
            Command::new("cl")
                .args(["/nologo", "/LD", "/std:c11", "/W4", "/WX"])
                .arg(format!("/Fo:{}", out_dir.join("provider.obj").display()))
                .arg(format!("/Fe:{}", path.display()))
                .args(if other {
                    vec!["/DOTHER_LIBRARY"]
                } else {
                    vec![]
                })
                .arg("provider.c")
                .arg("/link")
                .arg(format!(
                    "/IMPLIB:{}",
                    path.with_extension("dll.lib").display()
                ))
                .status()
        } else {
            Command::new("cc")
                .args([if macos { "-dynamiclib" } else { "-shared" }, "-fPIC"])
                .args(if other {
                    vec!["-DOTHER_LIBRARY"]
                } else {
                    vec![]
                })
                .args(["provider.c", "-o"])
                .arg(path)
                .status()
        }
        .unwrap();
        assert!(status.success());
    }
    println!("cargo:rustc-env=FIXTURE_LIBRARY={}", library.display());
    println!(
        "cargo:rustc-env=FIXTURE_OTHER_LIBRARY={}",
        other_library.display()
    );
    if env::var_os("CARGO_FEATURE_STRONG").is_some() {
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!(
            "cargo:rustc-link-lib=dylib={}",
            if windows {
                "linking_fixture.dll"
            } else {
                "linking_fixture"
            }
        );
        if windows {
            // Cargo adds deps to the test executable's DLL search path.
            let deps = out_dir.ancestors().nth(3).unwrap().join("deps");
            fs::create_dir_all(&deps).unwrap();
            fs::copy(&library, deps.join("linking_fixture.dll")).unwrap();
        } else {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", out_dir.display());
        }
    }
}
