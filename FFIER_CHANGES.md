# ffier Changes Needed for libkrun v2.0

Changes needed in the ffier framework (`~/Dev2/ffier/t/main/`) to support the libkrun v2.0 API design.

---

## 1. Drop `Handle` suffix from C type names

**Current**: `typedef void* KrunVmmBuilderHandle;`
**Wanted**: `typedef void* KrunVmmBuilder;`

The `Handle` suffix is redundant — `typedef void*` already communicates it's an opaque handle. All generated C names should drop it:

- Type names: `ExMyCalculatorHandle` → `ExMyCalculator`
- FfiHandle::C_HANDLE_NAME: `"ExMyCalculatorHandle"` → `"ExMyCalculator"`
- Header typedefs, function params, dyn_param dispatch constants — all affected

**Scope**: `ffier-macros/src/lib.rs` — change `handle_c_name` construction (currently `format!("{type_pfx}{struct_name}Handle")`). Also update `wrapper_c_handle` in `implementable`.

**Impact**: All generated headers change. RTTI TYPE_IDs change (hash of different string). Existing examples need updating.

---

## 2. Support `&[&str]` as a parameter type

**Current**: ffier handles `&str` → `KrunStr`, `&[u8]` → `KrunBytes`, `&Path` → `KrunPath`.
**Needed**: `&[&str]` → `const KrunStr* items, uintptr_t count`

When a Rust method takes `&[&str]`, the C signature should expand to two parameters:

```rust
// Rust
pub fn set_exec(&mut self, path: &str, args: &[&str]) -> Result<(), KrunError>;
```
```c
// Generated C
KrunError krun_vmmbuilder_set_exec(KrunVmmBuilder b, KrunStr path, const KrunStr* args, uintptr_t args_len);
```

**Implementation**:

1. Add a new `SliceKind::StrSlice` variant (or a new `ParamKind::StrSlice` / `ValueKind::StrSlice`)
2. Detect `&[&str]` in the type classifier:
   - `Type::Reference` → inner `Type::Slice` → inner `Type::Reference` → inner `Type::Path("str")`
3. FFI param: two params `(name: *const ffier::FfierBytes, name_len: usize)`
4. Conversion: iterate the `FfierBytes` array, convert each to `&str` via `as_str_unchecked()`
5. C type name for header: `"const KrunStr*"` + `"uintptr_t"` (two entries)
6. Also support `&[&[u8]]` → `const KrunBytes*` + `uintptr_t` (same pattern)
7. Also support `&[&Path]` → `const KrunPath*` + `uintptr_t`

**Header generation**: `build_header_line` needs to handle "multi-param" types that expand to two C parameters from one Rust parameter.

**C usage**:
```c
KrunStr args[] = { KRUN_STR("/bin/sh"), KRUN_STR("-c"), KRUN_STR("echo hello") };
CHECK(krun_vmmbuilder_set_exec(b, KRUN_STR("/bin/sh"), args, 3));
```

**Files**: `ffier-macros/src/lib.rs` (classify_ref_type, ffi_param_tokens, param_conversion, param_c_type_expr, build_header_line), `ffier/src/lib.rs` (FfierBytes already exists, no change needed there).

---

## 3. Support `BorrowedFd<'a>` as a parameter type

**Current**: ffier handles `OwnedFd` via `FfiType` (into_c/from_c converts to/from raw fd).
**Needed**: `BorrowedFd<'a>` → `int` in C, with the lifetime tying the fd to the struct's lifetime.

```rust
pub fn add_tty_port(&mut self, name: &str, fd: BorrowedFd<'a>) -> Result<(), KrunError>;
```
```c
KrunError krun_consoledevicebuilder_add_tty_port(KrunConsoleDeviceBuilder b, KrunStr name, int fd);
```

**Implementation options**:

A. **Implement `FfiType` for `BorrowedFd<'_>`** in the ffier crate:
```rust
impl FfiType for BorrowedFd<'_> {
    type CRepr = i32;
    const C_TYPE_NAME: &str = "int";
    fn into_c(self) -> i32 { self.as_raw_fd() }
    fn from_c(fd: i32) -> Self { unsafe { BorrowedFd::borrow_raw(fd) } }
}
```
This works because `BorrowedFd` is in `std` and `FfiType` is in `ffier` — orphan rules allow it since `FfiType` is local.

B. **Special-case in the proc macro** (like `&str`, `&Path`). Detect `BorrowedFd` and generate direct conversion.

Option A is simpler — just add the impl to `ffier/src/lib.rs` in the `std_impls` module alongside `OwnedFd`.

**Files**: `ffier/src/lib.rs` (add `FfiType for BorrowedFd<'_>` impl).

---

## 4. Builder pattern support: `Device::builder()` → `DeviceBuilder` → `.build()` → `Device`

**Current**: `#[ffier::exportable]` goes on a single impl block. The struct gets FfiType/FfiHandle.
**Needed**: Two linked structs where the builder produces the device.

This is already supported — just use `#[ffier::exportable]` on both:

```rust
#[ffier::exportable(prefix = "krun")]
impl<'a> ConsoleDeviceBuilder<'a> {
    pub fn add_tty_port(&mut self, ...) -> Result<(), KrunError>;
    pub fn build(self) -> Result<ConsoleDevice<'a>, KrunError>;
}

#[ffier::exportable(prefix = "krun")]
impl<'a> ConsoleDevice<'a> {}  // opaque, no methods
```

The `builder()` static method lives on `ConsoleDevice`:
```rust
#[ffier::exportable(prefix = "krun")]
impl<'a> ConsoleDevice<'a> {
    pub fn builder() -> ConsoleDeviceBuilder<'a>;
}
```

**No ffier changes needed** — this pattern works today. The `build(self)` consumes the builder handle (by-value self) and returns the device handle (via FfiType::into_c). Already supported.

---

## 5. Remove auto-generated `_create()` for types with `new()` / `builder()`

**Current**: ffier auto-generates `krun_foo_create()` calling `Default::default()` for types without lifetime params.
**Status**: Already fixed in recent commits — auto `create()` was removed. Static methods returning `Self` serve as constructors. ✓

No further changes needed.

---

## 6. `#[ffier::implementable]` improvements

**Current**: Basic vtable generation works (tested with krun example).
**Needed for v2.0**:

- **Vtable methods with `BorrowedFd` params** — needs item 3 above
- **Vtable methods returning `Result`** — classify return type as Result, generate proper error handling in the vtable dispatch
- **Better supertrait syntax** — the current `supers(TraitName { fn method(&self); })` works but is verbose

These are incremental improvements on the existing implementation, not blockers for the initial v2.0 API.

---

## 7. Multiple `#[ffier::exportable]` on the same struct

**Current**: Each `#[ffier::exportable]` generates `FfiType` + `FfiHandle` impls. Two `#[ffier::exportable]` on different impl blocks of the same struct → duplicate impl error.

**Needed**: For the builder pattern, `ConsoleDevice` might have:
```rust
#[ffier::exportable(prefix = "krun")]
impl<'a> ConsoleDevice<'a> {
    pub fn builder() -> ConsoleDeviceBuilder<'a>;
}

// Potentially a second impl block for other methods
```

**Solution**: Generate `FfiType`/`FfiHandle` impls only on the FIRST `#[ffier::exportable]` encountered, or use a marker attribute `#[ffier::exportable(prefix = "krun", no_impl)]` on secondary impl blocks to skip trait impl generation.

**Files**: `ffier-macros/src/lib.rs` — add `no_impl` option to `ReflectArgs`.

---

## 8. Configurable handle suffix (or no suffix)

**Current**: Handle C type name is hardcoded as `{Prefix}{StructName}Handle`.
**Needed**: Configurable or no suffix.

**Option A**: Remove suffix entirely (see item 1).
**Option B**: Make suffix configurable: `#[ffier::exportable(prefix = "krun", suffix = "")]`.

Option A is recommended — `Handle` suffix adds noise. The `typedef void*` already communicates the concept.

**Files**: `ffier-macros/src/lib.rs` — change `handle_c_name` from `format!("{type_pfx}{struct_name}Handle")` to `format!("{type_pfx}{struct_name}")`.

---

## 9. `SndDevice` removal

The `virtio-snd` device will be removed in libkrun v2.0 (pipewire backend was experimental). No ffier changes needed — just don't export it.

---

## 10. Separate C types for Str/Path (`const char*`) vs Bytes (`const uint8_t*`)

**Status: DONE** ✓

`KrunStr` and `KrunPath` now use `const char* data` (signedness-neutral, matches C string convention). `KrunBytes` uses `const uint8_t* data` (always unsigned). Previously all three were `const char*`.

- `KrunStr` — own struct: `{ const char* data; uintptr_t len; }`
- `KrunPath` — typedef of `KrunStr`
- `KrunBytes` — own struct: `{ const uint8_t* data; uintptr_t len; }`
- `KRUN_STR(s)` casts to `const char*`
- `KRUN_BYTES(arr)` casts to `const uint8_t*`

---

## Summary: Priority Order

| # | Change | Effort | Blocker? |
|---|--------|--------|----------|
| 1 | Drop `Handle` suffix | Small | Style, do first |
| 2 | `&[&str]` support | Medium | Yes — needed for `set_exec`, `set_env`, `set_port_map` |
| 3 | `BorrowedFd<'a>` support | Small | Yes — needed for console/serial/input devices |
| 7 | Multiple exportable on same struct | Small | Yes — needed for builder pattern |
| 4 | Builder pattern | None | Already works ✓ |
| 5 | No auto create() | None | Already fixed ✓ |
| 6 | implementable improvements | Medium | Not a blocker, incremental |
| 8 | Configurable suffix | Small | Same as #1 |
