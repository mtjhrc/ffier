# ffier

Automatic FFI binding generator for Rust libraries. Annotate your types with
proc macros and ffier generates:

- **C bridge** -- `extern "C"` functions + a C header (including doc comments),
  so your library can be called from C (or any language with C FFI support)
- **Rust client bindings** -- safe Rust wrappers that call through the C ABI
- **JSON schema** -- machine-readable description of the library's FFI surface,
  for building generators in other languages

The generated Rust bindings mirror the original library's API. Consumer code
can link against either the original Rust crate (native static linking) or the
generated C ABI wrapper (dynamic linking) by swapping a single `Cargo.toml`
dependency.

> [!CAUTION]
> **Early-stage project.** The API, ABI, and JSON schema are **not stable** and
> may change in the future. The end-to-end pipeline (annotated Rust code ->
> C header + Rust client bindings) works and is tested, but the internals were
> largely AI-generated and may contain bugs or need refactoring.

## Usage

### 1. Annotate your library

FFI annotations are gated behind a Cargo feature using standard `cfg_attr`,
so your crate compiles as pure Rust when FFI is not needed:

```rust
// Error types
#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(ffier::FfiError))]
pub enum CalcError {
    #[error("division by zero")]
    #[cfg_attr(feature = "ffi", ffier(code = 1))]
    DivisionByZero(),
}

// Exported types
pub struct Calculator;

#[cfg_attr(feature = "ffi", ffier::export)]
impl Calculator {
    pub fn new() -> Self { Self }
    pub fn add(&self, a: i32, b: i32) -> i32 { a + b }
    pub fn divide(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        if b == 0 { Err(CalcError::DivisionByZero()) } else { Ok(a / b) }
    }
}

// Register all exported types with a library prefix and stable type tags
#[cfg(feature = "ffi")]
ffier::library_definition!("mylib", library_tag = 1,
    Calculator = 1,
    CalcError = 2,
);
```

### 2. Generate bridge code

The bridge can be generated locally (same crate) or from a separate cdylib:

```rust
// Same crate (local mode):
#[cfg(feature = "ffi")]
__ffier_mylib_generate_ffi_bridge!(local);

// Or from a separate cdylib crate:
mylib::__ffier_mylib_generate_ffi_bridge!();
```

Building writes `target/ffier-mylib.json`. Feed that to the standalone
generators:

```bash
ffier-gen-c-header target/ffier-mylib.json > mylib.h
ffier-gen-rust-client target/ffier-mylib.json | rustfmt > src/generated.rs
```

### 3. Call from C

```c
#include "mylib.h"

MylibCalculator calc = mylib_calculator_new();
int32_t quotient;
MylibResult r = mylib_calculator_divide(calc, 10, 3, &quotient, NULL);
mylib_calculator_destroy(calc);
```

See [`tests/`](tests/) for the full test suite covering error types, builders,
lifetimes, `impl Trait` dispatch, file descriptors, and more.

## Features

- **`#[ffier::export]`** -- export struct methods, trait definitions, trait
  impls, enums, and free functions as C functions
- **`#[derive(FfiError)]`** -- export error enums with codes, messages, and
  optional data payloads (including `#[ffier(opaque)]` for non-marshallable
  fields like `anyhow::Error`)
- **`export_bitflags!`** -- wrap `bitflags!` invocations to export flag
  constants to C
- **`library_definition!`** -- register all exported types in one place with
  stable type tags and a shared library prefix
- Strings as `ptr + len` (no null-terminator copies)
- Builder pattern support (by-value self methods)
- Lifetime-preserving borrowed handles
- `&[&str]` and `&[&T]` slice parameters
- File descriptor passing (`OwnedFd`, `BorrowedFd`)
- Conditional compilation via standard `#[cfg_attr]` -- no FFI overhead when
  the feature is off

## String representation

Strings are represented as a `ptr + len` struct pair (e.g. `MylibStr`) rather
than null-terminated C strings.

With null-terminated strings, every FFI call that passes a Rust `&str` would
need to allocate a `CString`, copy the bytes, and append a null terminator.
With `ptr + len`, the pointer borrows directly from the source -- no
allocation, no copy, no ownership transfer.

A convenience macro (`PREFIX_STR(s)`, e.g. `MYLIB_STR(s)`) is generated in
the C header to construct these from string literals.

## How it works

The pipeline has two phases:

**Compile time** -- proc macros (`ffier-impl`) attach structured metadata to
your types via token streams. When bridge generation runs, the accumulated
metadata is parsed by the bridge generator (also in `ffier-impl`) which
produces `extern "C"` bridge functions and writes a JSON schema
(`target/ffier-{name}.json`) describing the library's FFI surface.

**Code generation** -- standalone tools (`ffier-gen-c-header`,
`ffier-gen-rust-client`) read the JSON schema and produce a C header or Rust
client bindings. The JSON schema is self-contained -- bindings can be generated
without access to the Rust source code. Third-party generators for other
languages can depend on `ffier-schema` to consume the same JSON, or
parse it directly.

```
your library            ffier-impl
(annotated Rust)  -->  (proc macros + bridge codegen)
                                |
                                |-- extern "C" bridge functions
                                |
                                `-- target/ffier-{name}.json
                                             |
                   ffier-gen-c-header  <-----+
                ffier-gen-rust-client  <-----+
     your generator (Go, Python, ...)  <-----'
```

## Crate structure

| Crate | Purpose |
|---|---|
| `ffier` | Facade -- re-exports `ffier-rt` + `ffier-impl` + `ffier-builtins` |
| `ffier-rt` | Runtime types (`FfierHandle`, `FfierBytes`, etc.) |
| `ffier-impl` | Proc macros (`#[export]`, `#[derive(FfiError)]`, `library_definition!`) + bridge codegen + metadata parsing |
| `ffier-builtins` | Pre-annotated traits (`PushStr`, `Error`) for built-in FFI protocols |
| `ffier-schema` | JSON schema types (`Library`, `Method`, etc.) -- depend on this to build third-party generators |
| `ffier-gen-c-header` | C header generator -- usable as a library (e.g. from `build.rs`) or as a standalone CLI |
| `ffier-gen-rust-client` | Rust client bindings generator -- usable as a library (e.g. from `build.rs`) or as a standalone CLI |

## Running tests

```bash
cargo test              # unit + integration tests
just check              # fmt + clippy + full test suite (incl. no-codegen build)
```
