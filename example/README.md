# ffier example

Demonstrates ffier's code generation pipeline with a small calculator library.

## Directory layout

```
example/
  mylib/              Rust library with ffier annotations
  mylib-cdylib/       cdylib bridge — builds a C-ABI shared library
  mylib-via-cdylib/   Rust client bindings that call through the C ABI
  rust-consumer/      Rust consumer — swappable between native and cdylib
  c-consumer/         C consumer — links against the cdylib
```

## How it works

1. **mylib** defines types (`Calculator`, `CalcResult`, `TextBuffer`) annotated
   with `#[ffier::exportable]`.

2. **mylib-cdylib** invokes ffier macros to generate C bridge functions and
   builds as a `cdylib` (shared library).  It also contains binaries for
   generating the C header (`gen-header`) and Rust client source
   (`gen-rust-client`).

3. **mylib-via-cdylib** contains generated Rust wrappers that call the C ABI
   functions from the cdylib.  The types mirror mylib's API.

4. **rust-consumer** uses a Cargo feature flag to choose between linking
   directly to `mylib` (native) or going through `mylib-via-cdylib` (cdylib).
   This lets you verify both paths produce identical behavior.

5. **c-consumer** generates a C header, compiles against the cdylib, and
   exercises the API from plain C.

## Quick start (from this directory)

```bash
# Run Rust consumer with native linking
just run-rust-native

# Build cdylib + run Rust consumer through C ABI
just run-rust-cdylib

# Build and run the C consumer
just run-c

# Regenerate Rust client bindings after changing mylib
just gen-rust-client
```
