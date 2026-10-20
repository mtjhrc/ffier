# Changelog

## Unreleased

### Breaking changes

- Preserve native Send/Sync contracts in exported traits and callbacks. Traits
  used in callback signatures require registered exported definitions. Concrete
  exported implementations of thread-safe traits require matching inherent
  `send`/`sync` annotations. Unsupported constraints and stronger use-site
  marker bounds now produce errors instead of being discarded.
- Require explicit `covariant`, `contravariant`, or `invariant` annotations
  for every exported opaque lifetime. Rust verifies claimed covariance and
  contravariance against the native type. Schema `lifetime_params` now contains
  ordered objects with `name` and `variance`; old name-only lists are rejected.
  Regenerate schemas and Rust clients together. C ABI does not change for
  variance representation.
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
- Generated Rust fieldless error variants now preserve native `Variant` or
  `Variant()` syntax instead of storing an error handle. Imported fieldless
  errors destroy their native handle immediately; callback returns create
  native errors to transfer to the caller. Fieldless `Display` uses static
  metadata instead of native formatting and creates no native handle. Rebuild
  bridges and regenerate schemas, headers, and bindings together for the new
  `error_new` symbol and variant metadata. Payload errors retain their native
  handles in `VariantData` wrappers.
- The C `{prefix}_error_payload` getter now accepts a field index after the
  error handle. Rust `FfiError::payload` also accepts an index; update manual
  implementations and calls. Rebuild callers with regenerated declarations.
  JSON error variants now use explicit `Fieldless`/`Tuple` data and
  `Typed`/`Opaque` fields instead of flags and a filtered field list.
  Accessing an opaque field, an out-of-bounds index, or a fieldless variant
  panics instead of silently leaving the output untouched. At the C ABI
  boundary, these panics abort the process.
- `result_name` and `result_name_cstr` now always return the raw variant name,
  without message overrides or payload suffixes. Use `result_message` for
  human-readable static text. Keep fieldless static messages and native
  `Display` consistent. Otherwise, native and generated callers show
  different text for the same error.

### Added

- Construct fieldless errors directly in generated Rust clients, including
  callback implementations shared with native Rust. `FfiError` derives accept
  unit variants and implement `from_code`; the native `{prefix}_error_new`
  factory rejects payload variants and unknown error types or codes.
- Inspect payload fields by native tuple index, including fields beside an
  opaque field. Field-level `#[ffier(opaque)]` hides one field; variant-level
  opacity applies it to every field. Generated Rust getters keep native indices.
- Look up static error messages through `{prefix}_result_message`, returning
  `FtStr` without an error handle or allocation. Generated fieldless `Display`
  writes this string directly; payload errors keep native formatting.
- Preserve explicit shared and mutable receiver lifetimes in metadata,
  schema, and native and generated method signatures. Retained local and
  foreign handles and returned values tie to the receiver borrow, and callback
  trampoline receivers must outlive their named borrows (`__T: Trait + 'lt`).

### Fixed

- Generate empty trait vtables with zero virtual method slots and only header
  plus drop callback. Native, C, and Rust generators use the same slot count;
  explicitly reserved slots still count when no methods remain.
- Retain registry types referenced only by typed error payload fields during
  schema pruning, including their alias, enum, and bitflags dependencies.
- Forward native error handles from generated Rust callbacks instead of boxing
  client wrappers as native handles. Preserve error type tags, variant codes,
  and ownership when returning library errors through callbacks.
- Preserve declared, compiler-verified variance for opaque client lifetimes.
  Invariant native lifetimes cannot shorten through shared client references
  and install shorter-lived borrows through interior-mutable native types.
- Preserve receiver mutability and the receiver tie of elided output
  lifetimes in extracted trait default helpers. `&mut self` defaults now
  extract to `__self: &'lt mut (impl Trait + ?Sized)` and elided returns
  borrow from the receiver, not from other parameters.

### Changed

- Reject typed `self` parameters (`self: &Self`), `'static` receivers on
  vtable traits (use a named method lifetime or `no_vtable`), lifetime bounds
  and where clauses other than `Self: Sized` on raw-handle methods, and named
  reference lifetimes on non-byte slice parameters. These contracts cannot be
  represented in the schema; they now fail at the macro instead of silently
  weakening generated client signatures.

## 0.2.0
