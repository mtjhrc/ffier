# Changelog

## Unreleased

### Breaking changes

- The C `{prefix}_error_payload` getter and Rust `FfiError::payload` now accept
  a field index. Update calls and manual implementations, then rebuild with
  regenerated declarations. JSON error variants use explicit `Fieldless`/`Tuple`
  data and `Typed`/`Opaque` fields, retaining opaque field positions.
  Accessing an opaque field, an out-of-bounds index, or a fieldless variant
  panics instead of silently leaving the output untouched. At the C ABI
  boundary, these panics abort the process.
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
  `_num_slots: u16`, `_reserved1: u16`, and `_reserved2: u32`. Count virtual
  method slots, including reserved gaps but excluding drop, rather than bytes.
  Drop remains present even when slot count is zero.
  Handle payloads now contain only the vtable and user-data pointers. Regenerate
  bindings and use the generated C `PREFIX_VTABLE(Type, ...)` macro to initialize
  slot count from the caller's compiled vtable type and zero the reserved fields.
- Reject all exported function and method names starting with `_`, including
  raw identifiers, with a compile-time diagnostic. This prefix is reserved for
  generated code. Users must rename affected functions and methods, including
  methods on traits exported with `no_vtable`.
- Replace trait schema field `max_vtable_slot` with `num_slots`. Users reading
  trait metadata must use the method-slot count directly instead of adding one
  to a highest index.

### Added

- Inspect payload fields by native tuple index, including fields beside an
  opaque field. Field-level `#[ffier(opaque)]` hides one field; variant-level
  opacity applies it to every field. Generated Rust getters keep native indices.

### Fixed

- Generate empty trait vtables with zero virtual method slots and only header
  plus drop callback. Native, C, and Rust generators use the same slot count;
  explicitly reserved slots still count when no methods remain.
- Forward native error handles from generated Rust callbacks instead of boxing
  client wrappers as native handles. Preserve error type tags, variant codes,
  and ownership when returning library errors through callbacks.

## 0.2.0
