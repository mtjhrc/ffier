# Conditional compilation

FFI metadata retains conditions even when their Rust items are disabled. Use
`ffier::export(cfg = "...")` for conditional exports and `#[cfg(...)]` for
conditional methods. Gate the FFI annotations themselves with `cfg_attr` when
the library also supports builds without FFI.

For example, export a function behind the `files` feature while keeping its
metadata available to the generators:

```toml
[features]
ffi = ["dep:ffier"]
files = []

[dependencies]
ffier = { version = "...", optional = true }
```

```rust
#[cfg_attr(feature = "ffi", ffier::export(cfg = "feature = \"files\""))]
#[cfg_attr(not(feature = "ffi"), cfg(feature = "files"))]
pub fn file_count() -> u32 {
    42
}

#[cfg(feature = "ffi")]
ffier::library_definition!("example", library_tag = 1,
    fn file_count,
);

#[cfg(feature = "ffi")]
ffier::generate_bridge!(local = __ffier_example_metadata,
    schema_output = "target/ffier-example.json");
```

With `ffi` enabled, `ffier::export` records the predicate and gates the Rust
function and its bridge. With `ffi` disabled, the second `cfg_attr` gates the
Rust function directly. In both builds, the function exists only when `files`
is enabled. Do not put an unconditional `#[cfg(feature = "files")]` before
the export annotation: that removes the item before ffier can record its
metadata.

Use the same pair for platform-specific exports:

```rust
#[cfg_attr(feature = "ffi", ffier::export(cfg = "unix"))]
#[cfg_attr(not(feature = "ffi"), cfg(unix))]
pub fn unix_value() -> i32 {
    7
}
```

Supported predicates are `windows`, `unix`, `target_os = "..."`, and
`feature = "..."`. Combine them with `all(...)`, `any(...)`, and
`not(predicate)`. Empty `all()` is true; empty `any()` is false. Other flags
and keys, including `target_arch` and `target_family`, are rejected.

## Schema representation

Conditions use structured objects instead of Rust source strings. Each object
has a `kind`; predicates with operands also have a `value`:

```json
{
  "kind": "all",
  "value": [
    { "kind": "unix" },
    { "kind": "feature", "value": "files" },
    { "kind": "not", "value": { "kind": "target_os", "value": "macos" } }
  ]
}
```

`all` and `any` take arrays. `not` takes one predicate. `target_os` and
`feature` take strings. `windows` and `unix` have no operand. An omitted
`cfg` field means unconditional availability.

This replaces the previous string representation. Regenerate schemas and
bindings together. Rust clients render the predicate tree as Rust `cfg`
attributes.

## C header guards

The C generator applies predicates to declarations, constants, and vtable
methods. Opaque handle typedefs remain available to other declarations.
Disabled methods retain reserved pointers so later vtable
slots keep their offsets. Constant-true predicates emit declarations without
guards. Constant-false predicates omit declarations. Boolean identity operands
and complementary predicates are folded before guards are emitted.

`windows` uses `_WIN32`; `unix` uses `__unix__` or `__APPLE__`. The generator
maps `target_os` values `windows`, `linux`, `macos`, `freebsd`, `openbsd`,
`netbsd`, and `android` to compiler macros. Linux guards exclude Android;
macOS guards exclude Apple mobile targets.

Feature predicates use library-prefixed macros: prefix `ft` and feature
`files` produce `FT_CFG_FEATURE_FILES`. Define the macro when compiling a
consumer to select the matching API.
