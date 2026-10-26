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
Other predicates use `<PREFIX>_CFG_<KEY>_<VALUE>`; flags use
`<PREFIX>_CFG_<NAME>`, where `<PREFIX>` is the library's FFI prefix. Names are
uppercased and punctuation becomes underscores. For example, prefix `ft` with
`feature = "files"` produces `FT_CFG_FEATURE_FILES`, which the C consumer can
define when using that feature. The prefix keeps two headers with different
prefixes from colliding in the consumer's macro namespace. Conditional vtable
methods retain a reserved pointer-sized slot when disabled.

Predicates that are constant are folded before emission, so they never reach
the consumer as `#if 1`, `#if 0`, or a tautology such as `#if !(A || !(A))`.
An always-enabled item is emitted without any preprocessor guard and an
always-disabled item is omitted entirely. Empty `all`/`any` are constants, as
are combinators containing a decisive operand or a predicate alongside its
negation.

## Resolving predicates at generation time

`ffier-gen-c-header` can resolve cfg predicates when generating a header.
Declarations whose predicates resolve to true lose their guards; declarations
whose predicates resolve to false are omitted. Remaining predicates become
preprocessor guards.

```
--features files,logging        assume these features are on
--disable-features legacy-api   assume these features are off
--guard-features abi-v2         leave this feature guarded for the consumer
--unspecified-features MODE     enabled (default), disabled, or guard
--cfg target_os=windows         assume this cfg atom holds, repeatable
```

`--unspecified-features` applies only to features named in none of
`--features`, `--disable-features`, or `--guard-features`.

The default is `enabled`. Cargo features are intended to be additive: enabling
a feature adds API without changing existing API. Under that assumption,
enabling every feature describes the largest API surface. Extra declarations
are harmless while unused; consumers still need a library build that provides
the API they use.

Specify features that change an existing ABI explicitly. Choose either:

- Pass them to `--features` or `--disable-features` to match the library build.
  The header becomes specific to that library build.
- Pass them to `--guard-features` to keep the decision with the consumer.
  The consumer must select the feature state that matches the library build.

For example, this feature changes the signature of an existing symbol:

```rust
#[cfg(feature = "wide")]      fn width(&self, i: i64) -> i64;
#[cfg(not(feature = "wide"))] fn width(&self, i: i32) -> i32;
```

Resolving `wide` either way emits one signature and omits the other. Using
that header with a library built the other way gives an ABI mismatch.

```sh
ffier-gen-c-header schema.json --features wide           # built with wide
ffier-gen-c-header schema.json --disable-features wide   # built without wide
ffier-gen-c-header schema.json --guard-features wide     # keep the guard
```

`--unspecified-features guard` leaves unspecified features guarded for the
consumer. Explicitly enabled or disabled features still resolve.
`--unspecified-features disabled` assumes unspecified features are off, for
example when describing a library build without optional features.

Platform predicates are guarded by default, so one header still serves every
target. Asserting them opts into a resolved header:

```
ffier-gen-c-header schema.json --cfg target_os=windows
```

Asserting one value of a single-valued key (`target_os`, `target_family`,
`target_arch`, `target_pointer_width`) rules out its siblings, and a target
description implies the `unix` or `windows` flag, so a single assertion can
collapse the platform guards too. An assertion applies to atoms the caller
names; it never forces a feature decision.

Rust clients always enforce platform predicates in both linking modes.
Strong linking also enforces Cargo feature predicates. Dynamic lookup allows
the consumer to discover feature-gated functions at runtime without enabling
the producer's Cargo features locally; this does not relax platform checks.
When multiple signatures share one C symbol, both linking modes enforce the
feature predicates that select a signature, including shared conditions that
constrain those choices. Independent availability conditions remain discoverable
dynamically. The client must enable features that match the producer's selected
signature.

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
