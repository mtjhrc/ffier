# ffier example

## Directory layout

```
example/
  mylib/                 Your Rust library, annotated with ffier
  mylib-cdylib/          Produces a .so with extern "C" wrappers, plus a C header
  mylib-via-cdylib/      Generated Rust wrappers that call mylib through the .so
  rust-consumer/         Your Rust program — can link mylib natively or through the .so
  c-consumer/            Your C program — calls mylib through the .so
```

The generated Rust wrappers in `mylib-via-cdylib` mirror the original
`mylib` API exactly. This means consumer code can swap between native Rust
linking and dynamic C ABI linking by changing a single `Cargo.toml`
dependency — the code itself stays identical.

## Step by step

### 1. Annotate your library (`mylib/`)

This is a regular Rust library. You annotate the types and methods you want
to make available through C with `#[ffier::exportable]`, and error types with
`#[derive(ffier::FfiError)]`:

```rust
#[derive(ffier::FfiError)]
pub enum CalcError {
    #[ffier(code = 1)]
    DivisionByZero,
}

#[ffier::exportable]
impl Calculator {
    pub fn new() -> Self { ... }
    pub fn divide(&self, a: i32, b: i32) -> Result<i32, CalcError> { ... }
}
```

These annotations attach metadata to your types. No C code is generated at
this point — the library remains pure Rust and can be used as a normal Rust
dependency.

### 2. Build the cdylib (`mylib-cdylib/`)

This crate produces a `.so` (or `.dylib` / `.dll`) shared library that
exposes your Rust API as C functions. It contains very little hand-written
code — just macro invocations that tell ffier which types to export and what
C name prefix to use:

**`src/lib.rs`** — each line generates the `extern "C"` bridge functions for
one type:

```rust
mylib::__ffier_meta_calculator!("mylib", ffier_gen_c_macros::generate_bridge);
mylib::__ffier_meta_calc_error!("mylib", ffier_gen_c_macros::generate_bridge);
```

The prefix `"mylib"` controls the C naming: `Calculator::divide` becomes
`mylib_calculator_divide`. The bridge functions handle type conversion,
handle boxing/unboxing, and error marshalling automatically.

This crate also contains two small binaries for generating source artifacts:

**`src/gen_header.rs`** — prints the C header to stdout. Each bridge macro
also generates a `__header()` function that returns the C declarations for
that type:

```rust
fn main() {
    let header = ffier_gen_c::HeaderBuilder::new("MYLIB_H")
        .add(mylib_calculator__header())
        .add(mylib_calc_error__header())
        .build();
    print!("{header}");
}
```

**`src/gen_rust_client.rs`** — prints Rust client binding source to stdout
(see next step).

### 3. Generated Rust client (`mylib-via-cdylib/`)

Contains generated source (`src/generated.rs`) produced by `gen-rust-client`.
You check this file in but never edit it by hand — regenerate with
`just gen-rust-client` after changing `mylib`.

This crate depends only on `ffier` (for FFI protocol types), not on `mylib`.
It links against the `.so` at runtime via a build script.

### 4. Rust consumer (`rust-consumer/`)

A Cargo feature flag selects the backend:

```toml
[features]
default = ["native"]
native = ["dep:mylib"]                 # link Rust code directly
via-cdylib = ["dep:mylib-via-cdylib"]  # call through the .so
```

```rust
#[cfg(feature = "native")]
use mylib as api;
#[cfg(feature = "via-cdylib")]
use mylib_via_cdylib as api;

use api::Calculator;
```

### 5. C consumer (`c-consumer/`)

A C program that includes the generated header and links against the `.so`.
The Makefile takes care of running `cargo build`, generating the header, and
compiling.

## Commands (from this directory)

```bash
just run-rust-native     # Run Rust consumer with direct Rust linking
just run-rust-cdylib     # Run Rust consumer through the .so
just run-c               # Build and run the C consumer
just gen-rust-client     # Regenerate mylib-via-cdylib/src/generated.rs
just clean               # Remove build artifacts
```
