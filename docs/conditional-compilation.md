# Conditional compilation

Methods in an exported impl or trait can use ordinary Rust `#[cfg(...)]`
attributes. The metadata retains every annotated method, including methods
disabled on the machine producing the schema.

For an entire exported item, use `#[ffier::export(cfg = "predicate")]`.
Rust removes an item with an outer `#[cfg]` before invoking its export macro;
passing the predicate to `export` lets ffier retain its metadata while gating
the Rust implementation. Gate its `library_definition!` entry with the same
predicate when necessary.

The JSON schema stores predicates as objects, never Rust source strings:

```json
{"cfg": {"target_os": "windows"}}
{"cfg": {"unix": true}}
{"cfg": {"all": [{"unix": true}, {"feature": "files"}]}}
{"cfg": {"not": {"any": [{"windows": true}, {"target_os": "macos"}]}}}
```

Each predicate object has one key. A flag has value `true`; a key/value
predicate has a string value. `all` and `any` contain arrays of predicates;
`not` contains one predicate. Empty `all` is true and empty `any` is false.
An absent `cfg` field means unconditional availability. This representation
also supports custom cfg names without changing the schema's Rust types.

The C generator translates Windows predicates to `defined(_WIN32)` (the
compiler macro used on both 32-bit and 64-bit Windows), and Unix, common OS,
architecture, and pointer-width predicates to corresponding compiler macros.
Other predicates use `FFIER_CFG_<KEY>_<VALUE>`; flags use `FFIER_CFG_<NAME>`.
Names are uppercased and punctuation becomes underscores. For example,
`feature = "files"` uses `FFIER_CFG_FEATURE_FILES`, which the C consumer can
define when using that feature. Conditional vtable methods retain a reserved
pointer-sized slot when disabled.

Rust clients always enforce platform predicates in both linking modes.
Strong linking also enforces Cargo feature predicates. Dynamic lookup allows
the consumer to discover feature-gated functions at runtime without enabling
the producer's Cargo features locally; this does not relax platform checks.
When multiple signatures share one C symbol, both linking modes enforce their
feature predicates to select a single ABI. The client must enable the features
that match the producer's selected signature.

Within one exported impl or trait, alternate methods can have the same Rust
name and different target-specific signatures. Alternate trait methods use
the same `ffier(index = N)` to share an ABI slot. Give each variant an explicit
cfg predicate; Rust rejects configurations that enable overlapping methods.

Same-named free-function variants stay in the same module and use the same
`export(cfg = "...")` mechanism as feature-gated exports. Register the name once:

```rust
#[ffier::export(cfg = "unix")]
pub fn resource(fd: std::os::fd::BorrowedFd<'_>) -> std::os::fd::RawFd {
    use std::os::fd::AsRawFd;
    fd.as_raw_fd()
}

#[ffier::export(cfg = "windows")]
pub fn resource(handle: std::os::windows::io::BorrowedHandle<'_>) -> std::os::windows::io::RawHandle {
    use std::os::windows::io::AsRawHandle;
    handle.as_raw_handle()
}

ffier::library_definition!("example", library_tag = 1, fn resource);
```

Ordinary `#[cfg(unix)]` / `#[cfg(windows)]` on adjacent exported definitions
also work. Both signatures appear in the schema under their shared C symbol
name, with their respective structured predicates. Methods use ordinary cfg
attributes inside one exported impl:

```rust
#[ffier::export]
impl Console {
    #[cfg(unix)]
    pub fn something_with_console(&self, fd: std::os::fd::BorrowedFd<'_>) { /* ... */ }

    #[cfg(windows)]
    pub fn something_with_windows(&self, handle: std::os::windows::io::BorrowedHandle<'_>) { /* ... */ }
}
```
