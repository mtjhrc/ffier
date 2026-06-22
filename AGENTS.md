# ffier

Rust FFI code generator. Annotate Rust types/traits with proc macros, generate C headers and Rust client bindings from a JSON schema.

## Stability

The library API, ABI, and JSON schema are **not stable**. Breaking changes to any of these are expected and acceptable.

## Architecture

```
ffier              lib — re-exports ffier-impl + ffier-rt + ffier-builtins
ffier-impl         proc-macro crate — all codegen
  src/lib.rs         #[export], #[derive(FfiError)], library_definition!, etc.
  src/meta.rs        metadata types (MetaMethod, etc.) + syn::Parse impls
  src/bridge.rs      extern "C" bridge generation + JSON schema emission
ffier-rt           runtime types (FfierHandle, FfierBytes, VtableHandle, etc.)
ffier-builtins     pre-annotated PushStr + Error traits
ffier-schema       JSON schema types (serde) — read by generators
ffier-gen-c-header     reads JSON → C header
ffier-gen-rust-client  reads JSON → Rust client bindings
```

### Conditional FFI

Users gate FFI annotations behind a Cargo feature using standard `cfg_attr`:

```rust
#[cfg_attr(feature = "ffi", ffier::export)]
impl Widget { ... }

#[cfg_attr(feature = "ffi", ffier::export)]
pub trait Processor {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn process(&self, input: i32) -> i32;
}

#[cfg_attr(feature = "ffi", ffier::export(reserved(1, 3), foreign))]
pub trait ExternalTrait { ... }

#[cfg(feature = "ffi")]
ffier::library_definition!("ft", library_tag = 1, Widget = 2, ...);
```

When the feature is off, all annotations are stripped by `cfg_attr` and
the crate compiles as pure Rust with no FFI overhead. The proc macro
understands `cfg_attr`-wrapped `#[ffier(...)]` attributes and unwraps
them during expansion.

`export_bitflags!` always defines the type (via `bitflags!`); FFI metadata
is unconditionally emitted but only referenced by `library_definition!`
which the user gates behind `#[cfg]`.

### Bridge generation

`library_definition!` emits a `__ffier_{prefix}_metadata!` macro
that drives a chain of metadata macros through each registered type,
accumulating metadata blobs. The chain's base case calls
`ffier::__generate_bridge` (a proc macro in ffier-impl) which produces
all `extern "C"` bridge functions and writes the JSON schema.

The bridge is generated via `ffier::generate_bridge!`:
```rust
// From the same crate as library_definition! (local):
ffier::generate_bridge!(local = __ffier_ft_metadata,
    schema_output = "../../target/ffier-ft.json");

// From a separate cdylib crate (external):
ffier::generate_bridge!(external = mylib::__ffier_ft_metadata,
    schema_output = "../../target/ffier-ft.json");
```

`schema_output` is relative to `CARGO_MANIFEST_DIR` (the crate calling the macro).

### Method kinds

Methods in the proc-macro token stream carry an explicit `method_kind` tag:

- `method_kind = definition` — trait definition method. Carries `index`, `has_default`, `raw_handle`.
- `method_kind = impl` — concrete method (struct impl or trait impl). Carries `ffi_name`, `is_builder`.

## Testing

```sh
cargo test          # runs all tests (170+)
cargo build         # full workspace build
```

You must run `just check` (fmt + clippy + tests) before every commit and ensure it passes.

## Git: AI Attribution

You must include an `Assisted-by` trailer identifying the tool and model used — e.g. `Assisted-by: OpenCode:claude-opus-4.6`. The trailer should appear before `Signed-off-by`.
