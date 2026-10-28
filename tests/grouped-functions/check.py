#!/usr/bin/env python3
"""Check libkrun-style platform functions from one inline export module."""
from pathlib import Path
import json
import os
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = Path(__file__).resolve().parent
TARGET = ROOT / "target" / "grouped-functions"
SCHEMA = ROOT / "target" / "ffier-grouped-functions.json"
WINDOWS = sys.platform == "win32"


def run(*args, **kwargs):
    command = [str(arg) for arg in args]
    print("+", subprocess.list2cmdline(command), flush=True)
    return subprocess.run(command, cwd=ROOT, check=True, **kwargs)


def cargo(action, manifest, *args, **kwargs):
    return run("cargo", action, "--manifest-path", manifest, "--target-dir", TARGET, *args, **kwargs)


def generate(package, destination, *args):
    with destination.open("w", encoding="utf-8", newline="\n") as output:
        run("cargo", "run", "--manifest-path", ROOT / "Cargo.toml", "-p", package,
            "--", SCHEMA, *args, stdout=output)


def main():
    manifest = FIXTURE / "Cargo.toml"
    run("cargo", "fmt", "--manifest-path", manifest, "--all", "--", "--check")
    for options in [[], ["--no-default-features"]]:
        cargo("clippy", manifest, "--all-targets", *options, "--", "-D", "warnings")
        cargo("test", manifest, *options)
    cargo("rustc", manifest, "--crate-type", "cdylib")

    schema = json.loads(SCHEMA.read_text())
    functions = schema["free_functions"]
    assert len(functions) == 2, functions
    assert {item["ffi_name"] for item in functions} == {"grouped_init_log"}
    assert {item["params"][0]["type"] for item in functions} == {"BorrowedFd", "BorrowedHandle"}
    assert {item["cfg"]["kind"] for item in functions} == {"unix", "windows"}
    assert all(item["params"][0]["optional"] for item in functions)
    assert all(item["doc"] for item in functions)

    client = TARGET / "client"
    (client / "src").mkdir(parents=True, exist_ok=True)
    generate("ffier-gen-rust-client", client / "src" / "generated.rs")
    generate("ffier-gen-c-header", TARGET / "grouped.h", "FFIER_GROUPED_H", "--fn-typedefs")
    if WINDOWS:
        run("cl", "/nologo", "/std:c11", "/W4", "/WX", "/TC", "/Zs", TARGET / "grouped.h")
    else:
        run("cc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-x", "c", "-fsyntax-only", TARGET / "grouped.h")

    (client / "Cargo.toml").write_text(f'''[package]
name = "grouped-functions-client"
version = "0.0.0"
edition = "2024"
[features]
ffier-strong = []
ffier-dynlookup = ["dep:libloading"]
[dependencies]
ffier = {{ package = "ffier-rt", path = {json.dumps(str(ROOT / "ffier-rt"))} }}
libloading = {{ version = "0.8", optional = true }}
[workspace]
''')
    (client / "build.rs").write_text('''fn main() {
    if std::env::var_os("CARGO_FEATURE_FFIER_STRONG").is_some() {
        let library = std::path::PathBuf::from(std::env::var_os("GROUPED_LIBRARY").unwrap());
        let folder = library.parent().unwrap();
        println!("cargo:rustc-link-search=native={}", folder.display());
        let windows = std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows";
        println!("cargo:rustc-link-lib=dylib={}", if windows { "ffier_test_grouped_functions.dll" } else { "ffier_test_grouped_functions" });
        if !windows { println!("cargo:rustc-link-arg=-Wl,-rpath,{}", folder.display()); }
    }
}
''')
    (client / "src" / "lib.rs").write_text('''include!("generated.rs");
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_logging_signature_uses_its_native_abi() {
        #[cfg(not(feature = "ffier-strong"))]
        unsafe {
            let library = libloading::os::unix::Library::new(env!("GROUPED_LIBRARY")).unwrap();
            set_library_handle(core::ptr::NonNull::new(library.into_raw())).unwrap();
            set_load_policy(LoadPolicy::Explicit).unwrap();
        }
        require(&[Symbol::GroupedInitLog]).unwrap();
        assert!(!init_log(None).unwrap());
        let file: &'static std::fs::File = Box::leak(Box::new(std::fs::File::open(std::env::current_exe().unwrap()).unwrap()));
        #[cfg(unix)]
        { use std::os::fd::AsFd; assert!(init_log(Some(file.as_fd())).unwrap()); }
        #[cfg(windows)]
        { use std::os::windows::io::AsHandle; assert!(init_log(Some(file.as_handle())).unwrap()); }
    }
}
''')
    suffix = ".dll" if WINDOWS else ".dylib" if sys.platform == "darwin" else ".so"
    name = ("" if WINDOWS else "lib") + "ffier_test_grouped_functions" + suffix
    environment = os.environ.copy()
    environment["GROUPED_LIBRARY"] = str(TARGET / "debug" / name)
    environment["PATH"] = str(TARGET / "debug") + os.pathsep + environment.get("PATH", "")
    modes = ["ffier-strong"] if WINDOWS else ["ffier-strong", "ffier-dynlookup", "ffier-dynlookup,ffier-strong"]
    for mode in modes:
        cargo("test", client / "Cargo.toml", "--features", mode, env=environment)

    rejected = TARGET / "out-of-line"
    (rejected / "src").mkdir(parents=True, exist_ok=True)
    (rejected / "Cargo.toml").write_text(f'''[package]
name = "grouped-functions-out-of-line"
version = "0.0.0"
edition = "2024"
[dependencies]
ffier = {{ path = {json.dumps(str(ROOT / "ffier"))} }}
[workspace]
''')
    (rejected / "src" / "lib.rs").write_text("#[ffier::export]\npub mod external;\n")
    (rejected / "src" / "external.rs").write_text("")
    failure = subprocess.run(["cargo", "check", "--manifest-path", str(rejected / "Cargo.toml"),
                              "--target-dir", str(TARGET)], cwd=ROOT, text=True, capture_output=True)
    assert failure.returncode != 0
    assert "inline" in failure.stderr, failure.stderr
    print("Inline grouped-function checks passed.")


if __name__ == "__main__":
    main()
