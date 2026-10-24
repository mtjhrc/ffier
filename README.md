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
ffier::generate_bridge!(local = __ffier_mylib_metadata,
    schema_output = "../../target/ffier-mylib.json");

// Or from a separate cdylib crate:
ffier::generate_bridge!(external = mylib::__ffier_mylib_metadata,
    schema_output = "../../target/ffier-mylib.json");
```

`schema_output` is the JSON schema path, relative to `CARGO_MANIFEST_DIR`.
Feed it to the standalone generators:

```bash
ffier-gen-c-header target/ffier-mylib.json > mylib.h
ffier-gen-rust-client target/ffier-mylib.json | rustfmt > src/generated.rs
```

### Rust client linking (Unix)

One generation pass produces a single source file supporting both runtime
symbol resolution and ordinary linker-resolved calls on Linux and macOS.
Windows linking support is outside this iteration.

Declare these features in the **crate containing the generated bindings**;
they are not features of the generator:

```toml
[features]
default = ["dynlookup"]
dynlookup = ["dep:libloading"]
strong = []

[dependencies]
ffier = { version = "..." }
libloading = { version = "0.8", optional = true }
```

With the default `dynlookup` feature, configure one lookup source per generated
client. This setter fixes both the source and loading policy for the lifetime
of that client:

```rust
use my_client::{LoadPolicy, Symbol, require, set_library_handle};

// `handle` is a valid dlopen handle kept loaded for the rest of the program.
unsafe { set_library_handle(Some(handle), LoadPolicy::Lazy)?; }
```

The `load_policy` argument controls unloaded calls: `LoadPolicy::Explicit`
panics until the symbol is explicitly loaded with `require(&[Symbol::...])`;
`LoadPolicy::Lazy` tries to resolve and cache the symbol on first use, panicking
if lookup fails. Already loaded calls use their cached function pointer in
either policy. Each wrapper retains its resolved function before converting
owned arguments, and callback arguments share one prepared allocator per call.
Invocation reuses those pointers without checking the symbol caches again.

`Some(handle)` borrows the supplied handle, including handles opened without
`RTLD_GLOBAL`. The caller retains ownership and must keep its library loaded
for the remaining lifetime of the program. The setter is unsafe because an
explicit handle must be valid and match these bindings. `None` selects the
process-global namespace. That namespace can contain symbols from multiple
libraries; use an explicit handle to restrict the lookup source.

The setter succeeds once. Further calls return `SetLibraryError`, including
attempts to change just the loading policy. Probes, explicit loads, and lazy
loads all use the same configured source. Concurrent setters cannot mix sources.
Without a setter call, the first probe, explicit load, or unloaded call fixes
the source to the process-global namespace with `LoadPolicy::Explicit`:

```rust
require(&[Symbol::MylibCalculatorNew, Symbol::MylibCalculatorDestroy])?;
```

Explicit loading should include every symbol the operation needs, including
destructors and error helpers. `require()` skips initialized symbols and returns
`LoadError` on the first missing symbol. That error preserves the original
`libloading::Error` through its `source()`. `Symbol::is_available()` probes the
configured source without initializing the symbol cache, even in lazy mode.

Applications can select strong linking without regenerating:

```toml
my-client = { version = "...", default-features = false, features = ["strong"] }
```

Provide the usual linker configuration for your library, for example through
`build.rs`. Strong linking supports both shared and static libraries. It uses
direct `unsafe extern "C"` calls, with no runtime symbol lookup or function
pointer cache. The safe wrapper API and symbol identities stay the same.
`set_library_handle()` and `require()` are successful no-ops, and
`Symbol::is_available()` always returns `true`, even for schema entries excluded by `cfg`: this reflects the compile-time
linking contract, not a runtime probe. Missing referenced symbols are the
linker's responsibility. `LoadError` remains available but has no constructible
failure state in strong builds.

**Strong always takes precedence**, including when both features are enabled.
Intermediate libraries should generally depend on `my-client` without selecting
a linking mode. The final application can enable `strong` on that same dependency;
Cargo feature unification selects direct calls throughout the graph. In a pure
strong-only graph, `libloading` is absent. If an intermediate dependency enables
the default `dynlookup` feature, Cargo retains that feature and its optional
`libloading` dependency; enabling `strong` still selects the strong implementation.
Enabling neither feature produces a compile error.

Schema `cfg` predicates restrict the strong client's API. Runtime lookup clients expose
optional API regardless of those predicates so they can discover newer symbols
at runtime using `Symbol::is_available()` and load them with `require()`.

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

## Errors

`FfiError` exposes error codes, variant names, messages, and tuple payloads.
Generated Rust clients can construct fieldless errors with the same syntax
as native Rust, including in callbacks: `Error::InvalidInput` or `Error::InvalidInput()`.

Fieldless client `Display` uses `ffier(message = "...")`, falling back to the
variant name. Keep it consistent with native `Display` so callers see the same text:

```rust
#[error("invalid input")]
#[ffier(code = 3, message = "invalid input")]
InvalidInput,
```

Clients can inspect and forward received payload errors, but cannot yet
construct new ones. Mark a tuple field `#[ffier(opaque)]` to hide it, or mark
the variant opaque to hide all its fields. Other fields remain inspectable.
Payload errors use native `Display`, including details from opaque fields.

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
