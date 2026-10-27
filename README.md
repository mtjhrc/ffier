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

See [conditional compilation](docs/conditional-compilation.md) for supported
predicates and their schema representation.

### Rust client linking (Unix)

One generation pass emits a single source file with two linking modes. Declare
both features in the **crate containing the generated bindings**; neither is a
default:

```toml
[features]
ffier-dynlookup = ["dep:libloading"]
ffier-strong = []

[dependencies]
ffier = { package = "ffier-rt", version = "..." }
libloading = { version = "0.8", optional = true }
```

`ffier-dynlookup` resolves symbols at runtime through `libloading`, so a client
can run against a newer library than it was generated from. `ffier-strong`
calls `unsafe extern "C"` functions resolved by the linker, for shared and
static libraries. Enabling neither is a compile error; enabling both selects
`ffier-strong`.

With `ffier-dynlookup`, configure the lookup source once:

```rust
use my_client::{LoadPolicy, set_library_handle, set_load_policy};

// `handle` is a dlopen handle kept loaded for the rest of the program;
// `None` selects the process-global namespace.
unsafe { set_library_handle(Some(handle))?; }
set_load_policy(LoadPolicy::Explicit)?; // optional; Lazy is the default
```

`LoadPolicy::Lazy` resolves each symbol on first call and panics if lookup
fails. `LoadPolicy::Explicit` panics on unloaded calls until
`require(&[Symbol::...])` loads them, catching version mismatches up front.
Each setter succeeds once; later calls return `SetLibraryError`. Without
`set_library_handle`, lookups use the process-global namespace.
`Symbol::is_available()` probes without caching.

If the schema exports any vtable, the library must provide
`PREFIX_vtable_handle_new`. All vtables share this allocator; explicit and lazy
loading resolve it automatically.

Strong linking keeps the same API: both setters and `require()` succeed as
no-ops, and `Symbol::is_available()` returns `true`. Select it per application
without regenerating:

```toml
my-client = { version = "...", features = ["ffier-strong"] }
```

Provide the usual linker configuration, for example through `build.rs`.
Intermediate libraries should depend on `my-client` without a mode; Cargo
feature unification selects `ffier-strong` for the final graph. Schema `cfg`
predicates restrict the `ffier-strong` API only, so `ffier-dynlookup` clients
can still discover optional symbols at runtime.

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

### Fixed-layout structs

Annotate a public, non-generic `#[repr(C)]` struct that derives `Copy`, with
public named fields, then register it with `value` instead of a type tag:

```rust
#[cfg_attr(feature = "ffi", ffier::export)]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Packet {
    pub kind: u16,
    pub sequence: u32,
}

#[cfg(feature = "ffi")]
ffier::library_definition!("mylib", library_tag = 1, value Packet);
```

Fields may be FFI scalars (`i8`..`i64`, `u8`..`u64`, `isize`, `usize`, `bool`,
`f32`, `f64`), exported enums, exported bitflags, or other value structs used
inline. Nested structs are supported; pointers to structs *inside* a struct are
not. References, lifetimes, generics, `Option` fields, packed or aligned
layouts, empty structs, private fields, raw identifiers, and field-level `cfg`
are rejected at compile time. A missing `Copy` derive is also rejected: the
by-value ABI treats the struct as plain data, so it must not have drop glue.
Exported enums must use `#[repr(uN)]`/`#[repr(iN)]`.

The Rust struct is the C layout, so the generated header declares the matching
`typedef struct` and no conversion or layout check is emitted. Rust `bool` is C
`bool`, `isize`/`usize` are `ssize_t`/`size_t`, and an enum field is the plain
repr integer on the wire. Values cross the ABI inline, or as `const T*`/`T*`
for `&T`/`&mut T`.

A value struct is only registered with `value`, never with a type tag, so it
cannot also be exported as an opaque handle.

### Optional value returns

A value struct can be returned by value, inside a `Result`, as an `Option`, or
both. Optional forms use an out-parameter plus a presence flag, because `None`
has to stay distinguishable from an output the callee never wrote.

See [value struct returns](docs/value-struct-returns.md) for all four
signatures, the write order, and the caller contract.

**Caller contract.** The bridge does not validate incoming values. An enum
field is the plain integer on the wire; Rust assumes it holds a declared
variant. A C caller writing any other value constructs an invalid Rust enum,
which is undefined behaviour. Pointers passed for `&T`/`&mut T` must be
non-null, aligned, initialized, valid for the whole call, and not aliased by
anything else. Validation may be added in a later version.

## Features

- **`#[ffier::export]`** -- export struct methods, trait definitions, trait
  impls, enums, free functions, and fixed-layout value structs as C functions
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

## Thread safety

See [Thread safety](docs/thread-safety.md) for handle declarations and callback
trait requirements.

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
