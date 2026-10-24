#!/usr/bin/env python3
"""Exercise Unix clients in separate Cargo invocations to avoid feature leakage."""

from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path(__file__).with_name("Cargo.toml")
TARGET = ROOT / "target" / "client-linking"


def cargo(*args, capture=False, success=True):
    command = ["cargo", args[0], "--manifest-path", str(MANIFEST)]
    if args[0] != "tree":
        command += ["--target-dir", str(TARGET)]
    command += list(args[1:])
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=capture)
    if (result.returncode == 0) != success:
        if capture:
            print(result.stdout, result.stderr)
        raise AssertionError(f"unexpected exit status: {command}")
    return result


# A default dynlookup client must be tested separately from the application, whose
# dependency enables strong. The fixture provider intentionally lacks optional API.
cargo("test", "-p", "linking-client")
for features in ("strong", "dynlookup,strong"):
    cargo("test", "-p", "linking-client", "--no-default-features", "--features", features)
    cargo("rustc", "-p", "linking-client", "--no-default-features", "--features", features,
          "--lib", "--", "--emit=llvm-ir")
    llvm = max((TARGET / "debug" / "deps").glob("linking_client-*.ll"),
               key=lambda path: path.stat().st_mtime_ns).read_text()
    assert "OnceLock" not in llvm
    assert "libloading" not in llvm
    assert re.search(r"declare\b[^\n]*@fixture_answer\(", llvm)
    for excluded in ("missing", "widget_new", "widget_newer", "worker_newer",
                     "optional_widget_new", "optional_widget_destroy", "optional_worker_value",
                     "optional_simple_child", "consume_fd", "consume_widget", "widget_consume",
                     "widget_consume_fd", "fd_worker_consume_fd"):
        assert f"@fixture_{excluded}(" not in llvm

# The pure strong dependency graph must not include even an unused libloading.
tree = cargo("tree", "-p", "linking-client", "--no-default-features", "--features", "strong",
             "-e", "normal", capture=True).stdout
assert "libloading" not in tree

# Separate edges activate dynlookup and strong on the same client crate.
cargo("test", "-p", "linking-application")

invalid = cargo("check", "-p", "linking-client", "--no-default-features",
                capture=True, success=False)
assert "generated ffier clients require the `dynlookup` or `strong` feature" in invalid.stderr

print("All Unix client linking configurations passed.")
