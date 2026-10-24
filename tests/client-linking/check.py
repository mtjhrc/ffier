#!/usr/bin/env python3
"""Exercise native clients in separate Cargo invocations to avoid feature leakage."""

from pathlib import Path
import json
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


def compile_llvm(stem, *args):
    result = cargo("rustc", "-p", "linking-client", "--message-format=json", *args,
                   "--", "--emit=llvm-ir", "-Copt-level=0", capture=True)
    # Use Cargo's artifact paths, rather than modification times: cached output
    # can be older than the IR for a different feature configuration.
    for line in result.stdout.splitlines():
        artifact = json.loads(line)
        if artifact.get("reason") == "compiler-artifact" and artifact["target"]["name"] == stem:
            for filename in artifact["filenames"]:
                path = Path(filename)
                candidate = path.with_name(path.stem.removeprefix("lib") + ".ll")
                if candidate.is_file():
                    return candidate.read_text()
    raise AssertionError(f"LLVM output missing for {stem}")


def function_bodies(llvm, name):
    # rustc emits demangled labels before definitions, independent of symbol hashes.
    return re.findall(r"^; " + re.escape(name) + r"\n(?:;[^\n]*\n)*define [^\n]*\{\n(.*?)^\}",
                      llvm, re.MULTILINE | re.DOTALL)


def symbol_fetches(body):
    return len(re.findall(r"^  [^;\n]*\b(?:call|invoke)\b[^\n]*__ffier_symbol", body, re.MULTILINE))


# A default dynlookup client must be tested separately from the application, whose
# dependency enables strong. The fixture provider intentionally lacks optional API.
cargo("test", "-p", "linking-client")
[answer] = function_bodies(compile_llvm("linking_client", "--lib"), "linking_client::answer")
assert symbol_fetches(answer) == 1
assert not re.search(r"\b(?:call|invoke)\b[^\n]*fixture_answer", answer)

# Callback conversion must use the prepared allocator without fetching it again.
conversions = function_bodies(compile_llvm("lazy", "--test", "lazy"),
                              "linking_client::Worker::__into_prepared_handle")
assert conversions
for conversion in conversions:
    assert symbol_fetches(conversion) == 0
    assert "__ffier_prepare" not in conversion
    assert "fixture_vtable_handle_new" not in conversion
    assert re.search(r"\b(?:call|invoke)\b[^\n]*%[^\s(]+\(", conversion)

for features in ("strong", "dynlookup,strong"):
    cargo("test", "-p", "linking-client", "--no-default-features", "--features", features)
    llvm = compile_llvm("linking_client", "--no-default-features", "--features", features, "--lib")
    [answer] = function_bodies(llvm, "linking_client::answer")
    assert re.search(r"\bcall\b[^\n]*@fixture_answer\(", answer)
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

print("All native client linking configurations passed.")
