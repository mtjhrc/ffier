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

Annotate a public, non-generic struct with named public fields and `#[repr(C)]`,
then register it with `value` instead of a type tag:

```rust
#[cfg_attr(feature = "ffi", ffier::export)]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Packet {
    pub kind: u16,
    pub sequence: u16,
    pub value: u32,
}

#[cfg(feature = "ffi")]
ffier::library_definition!("mylib", library_tag = 1, value Packet);
```

Fields may contain FFI scalars (`i8` through `i64`, `u8` through `u64`,
`isize`, `usize`, `bool`, `f32`, `f64`), registered enums, bitflags, and other
registered value structs. Field order and widths are preserved. Packed or
custom-aligned layouts, empty structs, raw field identifiers, field-level
conditional compilation, and optional fields are rejected. Gate the entire
struct with `#[ffier::export(cfg = "...")]` when needed.

Value structs cross the ABI inline or as pointers, without handles or
heap allocation. `&T` becomes `const T*`, `&mut T` becomes `T*`,
and optional references use a nullable pointer. Enum fields use their integer
wire representation and are validated before constructing or borrowing Rust
values. Borrowed pointers must be aligned, initialized, valid for the borrow,
and exclusively accessible for a mutable borrow.

Optional values are supported in returns, including `Result` success values:

Here `T` denotes its generated C value type; `Status` and `Error` denote
the library-prefixed result and error types, such as `MylibResult` and
`MylibError`.

| Rust return | C return | Additional C parameters |
| --- | --- | --- |
| `T` | `T` | none |
| `Option<T>` | `bool` | `T*` |
| `Result<T, E>` | `Status` | `T*, Error*` |
| `Result<Option<T>, E>` | `Status` | `bool*, T*, Error*` |

`Some` writes the value output; `None` leaves it untouched. A successful
`Result<Option<T>, E>` writes the presence flag, and writes the value only
when present. Errors leave both success outputs untouched. Callers must check
the result status before reading the presence flag, then check the flag
before reading the value. Generated Rust clients follow this order and use
`MaybeUninit` for value storage. Supply valid output pointers for outputs
that can be written. Library bridge calls permit a null `err_out`; a foreign
callback returning an error must supply the owned generated error handle to
the non-null error output provided by its Rust caller. These conventions also
apply to trait vtable slots. Optional value parameters and value slices are
currently rejected.

Importing a raw error handle through `Error::from_ffi` is unsafe: the handle
must be valid, uniquely owned, and match the result's native type and variant.
Generated `Result` wrappers perform this import internally.

When definitions live in another crate's submodule, re-export them at that
crate's root with `#[ffier::reexport]`. Use the same annotation on the library
crate's public re-export before registration:

```rust
// In the defining crate:
#[cfg_attr(feature = "ffi", ffier::reexport)]
pub use packets::Packet;

// In the library crate:
#[cfg_attr(feature = "ffi", ffier::reexport)]
pub use packet_types::Packet;
```

This forwards metadata, type-resolution helpers, default trait methods, and
derived error-trait metadata. The library registers the original types and
traits without redeclaring them. The bridge can also be generated from a
separate cdylib crate with `generate_bridge!(external = ...)`.

## Features

- **`#[ffier::export]`** -- export struct methods, trait definitions, trait
  impls, enums, free functions, and fixed-layout value structs
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

## Callback ownership and lifetimes

`impl Backend + 'static` transfers an owned callback and requires a value
without expiring borrows. `impl Backend + 'a` can own borrowed data; any
retaining object must remain within `'a`. Trait arguments (`Backend<'a>`),
value bounds (`+ 'a`), and reference lifetimes are separate schema fields.
Generated Rust clients preserve supported bounds and track callback handle
lifetimes. Raw ownership transfer is unsafe.
Callback handles carry trait tags and ownership kinds. Safe factory overrides
cannot substitute borrowed handles for owned handles or shared handles for
mutable handles.

`&impl Backend` and `&mut impl Backend` borrow a callback for the call.
Generated clients keep temporary callback handles alive until return and
destroy only their shells. They never destroy the borrowed callback value.
Explicit named reference lifetimes are currently rejected because these
temporary handles cannot support retention beyond the call. Borrowed callbacks
with lifetime-bearing returns are rejected too. Callback vtables with trait
lifetime parameters are also rejected; lifetime-parameterized
traits remain usable through concrete dispatch. Higher-ranked bounds,
additional trait bounds, and method lifetime bounds/where clauses are rejected
instead of discarded. Foreign traits and unsupported dynamic forwarding
require concrete dispatch.

Opaque client handles keep every lifetime parameter invariant because hidden
Rust fields may require it. Signatures with named receiver lifetimes, typed
receivers, constrained method lifetimes, or named reference lifetimes on
non-byte slices are rejected when the schema cannot preserve their contracts.
Byte-buffer references preserve their named lifetimes.

C callers must enforce lifetimes themselves:

- Pass stack `PREFIX_VTABLE_HANDLE` values only to borrowed parameters.
- Owned parameters consume a heap handle with ffier's owned handle layout.
  Allocation must be compatible with Rust's allocator. Do not pass stack
  handles or borrowed shells as owned callbacks.
- After ownership transfer, do not use or destroy the transferred handle.
  Library destroys it exactly once when released, not necessarily at return.
- Keep `vtable_ptr`, `user_data`, and borrowed callback data valid until release.
  Library calls the vtable `drop` callback exactly once if present. That callback
  releases `user_data`; it must not free the consumed handle shell again.
- Borrowed callbacks must not be retained, destroyed, or used after call return.
- `+ 'static` forbids expiring borrows. Other bounds require release before
  borrowed data expires.

Mutable byte buffers (`&mut [u8]`) use a separate writable pointer-and-length
representation (`FfierMutBytes`, `PrefixBytesMut` in generated C headers).
They can be passed to exported methods and callbacks without copying. C callers
must provide initialized writable bytes, with a non-null pointer for nonempty
buffers and a length no greater than Rust's `isize::MAX`. Keep the storage valid
for reads and writes and exclusively accessible for the call and for every
active Rust borrow, including borrows returned or retained by the API. A
zero-length buffer may use a null pointer.

## Thread safety

Fixed-layout value structs always require `Send + Sync`. Their scalar, enum,
bitflags, and nested value fields give generated client structs both auto traits.

Opaque handle types opt in on their inherent export:

```rust
#[cfg_attr(feature = "ffi", ffier::export(send, sync))]
impl Widget {
    // ...
}
```

Use `send`, `sync`, or both independently. The Rust compiler verifies the actual
type, including all lifetime parameters. The schema records these guarantees;
generated client handles implement only the declared auto traits. Unannotated
handles remain non-`Send`, non-`Sync`, even when their hidden Rust value happens
to satisfy those bounds. Specialized or constrained impls cannot declare
type-wide guarantees.

Exported traits use native supertraits, not export arguments:

```rust
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait Backend: Send + Sync {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn poll(&mut self) -> i32;
}
```

These bounds apply to every implementation, including generated client
callbacks. Every trait used in an exported `impl Trait` callback signature must
have an exported definition registered in `library_definition!`, so its native
bounds and lifetimes are available to the generators. Use `no_vtable` for traits
that support only concrete dispatch. Trait-impl-only registration remains usable
for method bindings, but does not define a callback contract.

Concrete client types implementing the trait must also declare the required
`send`/`sync` guarantees on their inherent export. Stronger use-site
trait bounds (`impl LocalBackend + Send`) remain unsupported: put requirements
on the exported trait. Redundant marker bounds remain in the schema; generated
signatures rely on the trait's equivalent supertraits. Combining thread-safety
markers with other supertraits or where clauses is currently rejected.

`Send` permits ownership transfer and destruction on another thread. `Sync`
permits concurrent shared access, never simultaneous mutable access. Shared
borrowed callbacks can move between threads only for `Sync` traits. Dynamic
shared forwarding for `Send`-only traits requires `Sync` or concrete dispatch.
Lifetime bounds remain separate; `Send` never implies `'static`.

C callbacks must uphold the trait's guarantees for `user_data`, vtable storage,
callback functions, and destruction. Vtable storage remains valid and immutable
throughout callback use. Generic raw handles and `VtableHandle` do not gain
`Send`/`Sync`; unsafe callback construction must uphold the exact declared
contract.

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
