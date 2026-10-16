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

## 0.2.0
