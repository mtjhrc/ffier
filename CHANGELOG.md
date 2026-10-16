# Changelog

## Unreleased

### Breaking changes

- Reject conflicting FFI type names during bridge generation instead of
  silently overwriting schema entries. Exported types and traits must have
  unique names within a library, including names from other modules or crates
  and framework types. Users with an error enum named `Error` must rename it
  if their library also registers the built-in `ffier_builtins::Error` trait.
- Require absolute crate paths starting with `::` in `foreign` and
  `foreign_return` annotations, for example `foreign = ::other_lib`. Users must
  update relative annotation paths to avoid module shadowing and ambiguous
  registry comparisons.
- Generated Rust fieldless error variants now preserve native `Variant` or
  `Variant()` syntax instead of storing an error handle. Imported fieldless
  errors destroy their native handle immediately; formatting and callback
  returns create temporary or transferred native errors. Rebuild bridges and
  regenerate schemas, headers, and bindings together for the new `error_new`
  symbol and variant metadata. Payload errors retain their native handles.

### Added

- Construct fieldless errors directly in generated Rust clients, including
  callback implementations shared with native Rust. `FfiError` derives accept
  unit variants and implement `from_code`; the native `{prefix}_error_new`
  factory rejects payload variants and unknown error types or codes.

### Fixed

- Forward native error handles from generated Rust callbacks instead of boxing
  client wrappers as native handles. Preserve error type tags, variant codes,
  and ownership when returning library errors through callbacks.

## 0.2.0
