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
- Move vtable length from each handle into an 8-byte vtable header:
  `_vtable_len: u16`, `_reserved1: u16`, and `_reserved2: u32`. Length counts
  function-pointer slots, including drop and reserved gaps, rather than bytes.
  Handle payloads now contain only the vtable and user-data pointers. Regenerate
  bindings and use the generated C `PREFIX_VTABLE(Type, ...)` macro to initialize
  length from the caller's compiled vtable type and zero the reserved fields.
- Reject all exported function and method names starting with `_`, including
  raw identifiers, with a compile-time diagnostic. This prefix is reserved for
  generated code. Users must rename affected functions and methods, including
  methods on traits exported with `no_vtable`.

### Fixed

- Forward native error handles from generated Rust callbacks instead of boxing
  client wrappers as native handles. Preserve error type tags, variant codes,
  and ownership when returning library errors through callbacks.

## 0.2.0
