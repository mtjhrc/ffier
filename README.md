# ffier

> **Status: unfinished proof of concept.**

Automatic FFI binding generator for Rust libraries. Annotate your types with
`#[ffier::exportable]` and ffier generates:

- **C bridge** — `extern "C"` functions + a C header (`ffier-gen-c`)
- **Rust client bindings** — safe wrappers that call through the C ABI, enabling
  consumers to swap between native Rust linking and dynamic C ABI linking by
  changing a single `Cargo.toml` dependency (`ffier-gen-rust`)

The architecture is extensible: generators for other languages (Go, Python, ...)
can be added as separate crates by depending on `ffier-meta`.

## Crate structure

| Crate | Purpose |
|---|---|
| `ffier` | Facade — re-exports `ffier-rt` + `ffier-annotations` |
| `ffier-rt` | Runtime types (`FfiType`, `FfierBytes`, `HeaderSection`, etc.) |
| `ffier-annotations` | Proc macros: `#[exportable]`, `#[derive(FfiError)]`, `#[implementable]` |
| `ffier-meta` | Metadata types + parsers — the extensibility point for third-party generators |
| `ffier-gen-c` | C bridge generator (`generate_bridge`) |
| `ffier-gen-rust` | Rust client source generator (`generate_client_source`) |

## Quick start

See [`example/`](example/) for a complete working example with a calculator, text
buffer, C consumer, and swappable Rust consumer.

```rust
// In your library crate:
#[ffier::exportable]
impl MyType {
    pub fn new() -> Self { ... }
    pub fn do_thing(&self, input: &str) -> Result<i32, MyError> { ... }
}

// In your cdylib crate:
my_lib::ffier_meta_op_my_type!("prefix", ffier_gen_c::generate_bridge);
```

## Running tests

```bash
cd tests && just        # runs everything: C tests, valgrind, miri, consumer tests
```

## Acknowledgements

This project was written with the assistance of Claude Code (Claude Opus 4.6)
and GPT-5.4 High via Cursor.
