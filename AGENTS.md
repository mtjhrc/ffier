# ffier

Rust FFI code generator. Annotate Rust types/traits with proc macros, generate C headers and Rust client bindings from a JSON schema.

## Stability

The library API, ABI, and JSON schema are **not stable**. Breaking changes to any of these are expected and acceptable.

## Architecture

```
ffier-annotations          proc macros (#[exportable], #[implementable], etc.)
  |-- uses ffier-meta        to parse/emit structured token streams
  `-- emits token streams -> consumed by ffier-bridge-macros::generate
                                        |
                              ffier-bridge  (uses ffier-meta to parse tokens)
                                |-- generates extern "C" bridge functions
                                `-- writes target/ffier-{name}.json  (ffier-schema types)
                                                    |
                              ffier-gen-c-header  <--+  (reads JSON schema)
                              ffier-gen-rust-client <-'
```

`ffier-meta` is a shared parsing library — both `ffier-annotations` and
`ffier-bridge` depend on it. It defines the metadata types (`MetaMethod`,
`MetaMethodContext`, etc.) and their `syn::Parse` impls. The token stream
is the data transport between annotations and bridge; `ffier-meta` is
not a relay but a shared vocabulary.

### Method kinds

Methods in the proc-macro token stream carry an explicit `method_kind` tag:

- `method_kind = definition` -- trait definition method (from `#[implementable]`). Carries `index`, `has_default`, `raw_handle`.
- `method_kind = impl` -- concrete method (from `#[exportable]` or `#[trait_impl]`). Carries `ffi_name`, `is_builder`.

In the JSON schema, `ffi_name` is always a top-level field on `Method`. Trait definition methods additionally have an optional `trait_definition` sub-object with `index` and `has_default`.

## Testing

```sh
cargo test          # runs all tests (170+)
cargo build         # full workspace build
```

You must run `just check` (fmt + clippy + tests) before every commit and ensure it passes.

## Git: AI Attribution

You must include an `Assisted-by` trailer identifying the tool and model used — e.g. `Assisted-by: OpenCode:claude-opus-4.6`. The trailer should appear before `Signed-off-by`.
