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
> may change in the future. The end-to-end pipeline (annotated Rust code →
> C header + Rust client bindings) works and is tested, but the internals were
> largely AI-generated and may contain bugs or need refactoring.

## Usage

### 1. Annotate your library

```rust
// Error types
#[derive(Debug, thiserror::Error, ffier::FfiError)]
pub enum CalcError {
    #[error("division by zero")]
    #[ffier(code = 1)]
    DivisionByZero(),
}

// Exported types
pub struct Calculator;

#[ffier::exportable]
impl Calculator {
    pub fn new() -> Self { Self }
    pub fn add(&self, a: i32, b: i32) -> i32 { a + b }
    pub fn divide(&self, a: i32, b: i32) -> Result<i32, CalcError> {
        if b == 0 { Err(CalcError::DivisionByZero()) } else { Ok(a / b) }
    }
}

// Register all exported types with a library prefix and stable type tags
ffier::library_definition!("mylib", library_tag = 1,
    Calculator = 1,
    CalcError = 2,
);
```

### 2. Create a cdylib crate

In a separate crate (`mylib-cdylib`), one line generates all `extern "C"`
bridge functions, the JSON schema, and the C header data:

```rust
mylib::__ffier_mylib_library!(ffier_bridge_macros::generate);
```

### 3. Generate bindings

Building the cdylib writes `target/ffier-mylib.json`. Feed that to the
standalone generators:

```bash
ffier-gen-c-header target/ffier-mylib.json > mylib.h
ffier-gen-rust-client target/ffier-mylib.json | rustfmt > src/generated.rs
```

### 4. Call from C

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

- **`#[exportable]`** -- export struct methods as C functions
- **`#[implementable]`** -- export traits with vtable-based dynamic dispatch
- **`#[trait_impl]`** -- export concrete trait implementations
- **`#[derive(FfiError)]`** -- export error enums with codes, messages, and
  optional data payloads (including `#[ffier(opaque)]` for non-marshallable
  fields like `anyhow::Error`)
- **`library_definition!`** -- register all exported types in one place with
  stable type tags and a shared library prefix
- Strings as `ptr + len` (no null-terminator copies)
- Builder pattern support (by-value self methods)
- Lifetime-preserving borrowed handles
- `&[&str]` and `&[&T]` slice parameters
- File descriptor passing (`OwnedFd`, `BorrowedFd`)

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

**Compile time** -- proc macros (`ffier-annotations`) attach structured
metadata to your types via token streams. When the cdylib crate is built,
`ffier-bridge` (invoked through `ffier-bridge-macros`) parses those token
streams, generates `extern "C"` bridge functions, and writes a JSON schema
(`target/ffier-{name}.json`) describing the library's FFI surface.

**Code generation** -- standalone tools (`ffier-gen-c-header`,
`ffier-gen-rust-client`) read the JSON schema and produce a C header or Rust
client bindings. The JSON schema is self-contained — bindings can be generated
without access to the Rust source code. Third-party generators for other
languages can depend on `ffier-schema` to consume the same JSON, or
parse it directly.

```
your library          ffier-annotations       ffier-bridge
(annotated Rust)  -->  (proc macros)      -->  (bridge codegen)
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
| `ffier` | Facade -- re-exports `ffier-rt` + `ffier-annotations` |
| `ffier-rt` | Runtime types (`FfiType`, `FfierBytes`, etc.) |
| `ffier-annotations` | Proc macros: `#[exportable]`, `#[implementable]`, `#[trait_impl]`, `#[derive(FfiError)]` |
| `ffier-builtins` | Pre-annotated traits (`PushStr`, `Error`) for built-in FFI protocols |
| `ffier-meta` | Internal: parses proc-macro token streams into structured metadata for `ffier-bridge` |
| `ffier-schema` | JSON schema types (`Library`, `Method`, etc.) -- depend on this to build third-party generators |
| `ffier-bridge` | Generates `extern "C"` bridge functions + writes JSON schema from metadata |
| `ffier-bridge-macros` | Proc macro entry point for bridge generation |
| `ffier-gen-c-header` | C header generator -- usable as a library (e.g. from `build.rs`) or as a standalone CLI |
| `ffier-gen-rust-client` | Rust client bindings generator -- usable as a library (e.g. from `build.rs`) or as a standalone CLI |

## Running tests

```bash
cargo test              # unit + integration tests
just check              # fmt + clippy + full test suite
```
