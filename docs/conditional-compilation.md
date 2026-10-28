# Conditional compilation

FFI metadata retains conditions even when their Rust items are disabled. Use
`ffier::export(cfg = "...")` for conditional exports and `#[cfg(...)]` for
conditional methods. Gate the FFI annotations themselves with `cfg_attr` when
the library also supports builds without FFI.

To retain alternate signatures of the same function in one portable schema,
put them in an inline module annotated with `#[ffier::export]`:

```rust
#[cfg_attr(feature = "ffi", ffier::export)]
pub mod logging {
    #[cfg(unix)]
    pub fn target_present(target: Option<std::os::fd::BorrowedFd<'static>>) -> bool {
        target.is_some()
    }

    #[cfg(windows)]
    pub fn target_present(target: Option<std::os::windows::io::BorrowedHandle<'static>>) -> bool {
        target.is_some()
    }
}

#[cfg(feature = "ffi")]
ffier::library_definition!("example", library_tag = 1, fn logging::target_present);
```

The attribute exports direct public functions, grouping declarations with the
same name. Both signatures and their cfg predicates enter the schema; native
Rust and generated bindings select the matching signature when compiled.
Imports, private helpers, types, and nested modules keep their ordinary Rust
behavior. Register each function name once. With `ffi` disabled, the module is
ordinary Rust. Out-of-line declarations such as `mod logging;` are rejected:
the attribute receives its declarations directly and does not read source files.
Use cfg on the contained functions; module-level `export(cfg = ...)` is unsupported.

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
attributes. Platform predicates apply in both linking modes. Positive feature
predicates also allow dynamic lookup, including inside `all` and `any`.
Predicates inside `not` remain unchanged.

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

Unresolved feature predicates use library-prefixed macros: prefix `ft` and feature
`files` produce `FT_CFG_FEATURE_FILES`. Define the macro when compiling a
consumer to select the matching API.

## Resolve predicates while generating headers

Pass assumptions matching the library build to remove resolved guards:

```sh
ffier-gen-c-header target/ffier-example.json \
    --features files --cfg target_os=linux > example.h
```

- `--features files,logging` enables named features.
- `--disable-features files` disables named features.
- `--guard-features files` leaves named features for the consumer to select.
- `--unspecified-features enabled|disabled|guard` controls unlisted features.
  The default is `enabled`.
- `--cfg windows`, `--cfg unix`, or `--cfg target_os=linux` asserts a platform
  predicate. Repeat `--cfg` for multiple assumptions.
- `--guard unix` or `--guard target_os=linux` keeps a platform predicate
  guarded, even when `--cfg` would resolve it.

Feature lists accept repeated options or comma-separated names. Features must
not appear in more than one list. Platform assumptions must not contradict
one another. Use feature options for features; `--cfg` and `--guard` accept
only platform predicates.

Unlisted features default to enabled because Cargo features normally add API.
For features that change an existing ABI, select the state matching the library
build or keep the feature guarded. Use `--unspecified-features guard` to leave
all unlisted features for the consumer to select.

Platform predicates stay guarded unless asserted. Selecting one `target_os`
rules out other OS values. Known OS values resolve the matching `windows`
and `unix` flags. The `unix` flag does not identify one OS.

Resolved true predicates emit declarations directly. Resolved false predicates
omit declarations. Only unresolved predicates produce C guards. The Rust
`ffier_gen_c_header::generate` API returns `Result<String, String>` and reports
invalid assumptions as errors.
