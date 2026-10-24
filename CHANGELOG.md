# Changelog

## Unreleased

### Breaking changes

- Generate one Unix Rust client supporting default `dynlookup` runtime symbol
  loading and feature-selected `strong` linker calls. `strong` takes precedence
  when Cargo unifies both features. Client crates declare an optional
  `libloading` dependency; pure strong-only builds need no loader. Remove generator `Options`,
  `generate_with_options`, `generate_from_file_with_options`, and CLI `--weak`.
  A one-time unsafe `set_library_handle(handle, load_policy)` fixes the client's
  lookup source and `LoadPolicy::Explicit` or `LoadPolicy::Lazy` behavior for
  unloaded calls. Reject reconfiguration so explicit and lazy loads cannot mix
  library handles. Prepare symbols before transferring owned arguments and
  callbacks so failed lookup still releases Rust values. Reuse prepared callees
  and callback allocators instead of checking their symbol caches again. `require(symbols)`
  returns the shared `LoadError`; `Symbol::is_available()` probes the configured source.
  Without a setter, the first lookup fixes the process namespace and explicit
  loading policy. Strong builds retain the shared API as no-ops. Schema
  conditions restrict strong API while runtime lookup clients retain optional
  symbol discovery. Windows support remains a separate follow-up.

- Rename the C `PREFIX_VTABLE_HANDLE` macro to `PREFIX_TEMP_OBJ` and require
  owned vtable handles to come from the generated `PREFIX_vtable_handle_new`
  function. The macro builds a borrowed, call-scoped object that must only be
  passed to borrowed parameters; consuming an unowned handle now aborts at
  runtime instead of freeing foreign memory. Callers that hand-allocated a
  handle for an owned parameter, or relied on their allocator matching Rust's,
  must switch to the constructor. Generated Rust client bindings already do.
- Rename the handle metadata constants to state one fact each:
  `METADATA_OWNED` becomes `METADATA_FFIER_ALLOCATED` (who allocated the
  handle) and `METADATA_BORROWED` becomes `METADATA_INDIRECT` (payload is a
  pointer to a value owned elsewhere). `METADATA_ARRAY_ELEMENT` is removed;
  an object-array element is now `METADATA_INDIRECT` without
  `METADATA_FFIER_ALLOCATED`. A new `METADATA_DROP_VALUE` bit records whether
  the value's destructor is meaningful, so freeing the handle and dropping the
  value are separate decisions instead of one inferred from `METADATA_INDIRECT`.
  `METADATA_DEFAULT_DISPATCH` moves to bit 3 so its method index field starts
  at bit 4. Code reading the raw metadata field must use the new constants.
- Bound vtable method indices to `u16`, matching the `u16` slot count in the
  vtable header. `#[ffier(index = N)]` rejects larger values at compile time
  instead of truncating the generated slot count and the metadata index field.
  The default-dispatch metadata field is a `u16` index starting at bit 4, one
  bit above the default-dispatch flag.
- Move handle allocation and destruction out of `ffier-rt` into a new
  `ffier-bridge` crate. `ffier_rt` now holds only layouts, metadata bits, and
  read-only accessors, so it cannot allocate and cannot choose a mismatched
  allocator. `ffier-bridge` provides `ffier_handle_new`,
  `ffier_handle_new_borrowed`, `ffier_handle_consume`, `ffier_handle_drop`,
  and `ffier_object_array_free`; the `ffier` facade re-exports them, so
  `ffier::` call sites keep working, but `ffier_rt::` paths no longer resolve.
  `FfierObjectArray` gains `items`, `len`, and `is_empty` accessors for readers
  that must not free.
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

### Fixed

- Generate empty trait vtables with zero virtual method slots and only header
  plus drop callback. Native, C, and Rust generators use the same slot count;
  explicitly reserved slots still count when no methods remain.
- Retain registry types referenced only by typed error payload fields during
  schema pruning, including their alias, enum, and bitflags dependencies.
- Forward native error handles from generated Rust callbacks instead of boxing
  client wrappers as native handles. Preserve error type tags, variant codes,
  and ownership when returning library errors through callbacks.

## 0.2.0
