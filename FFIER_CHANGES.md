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

**Scope**: `ffier-macros/src/lib.rs` — change `handle_c_name` construction (currently `format!("{type_pfx}{struct_name}Handle")`). If `implementable` should keep the same naming scheme later, update `wrapper_c_handle` there too, but that is not required to unblock libkrun 2.0.

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
KrunError krun_libkruninitbuilder_set_exec(KrunLibkrunInitBuilder b, KrunStr path, const KrunStr* args, uintptr_t args_len);
```

**Implementation**:

1. Add a new `SliceKind::StrSlice` variant (or a new `ParamKind::StrSlice` / `ValueKind::StrSlice`)
2. Detect `&[&str]` in the type classifier:
   - `Type::Reference` → inner `Type::Slice` → inner `Type::Reference` → inner `Type::Path("str")`
3. FFI param: two params using the existing per-element `&str` FFI representation plus a length
4. Conversion: iterate the `FfierBytes` array, convert each to `&str` via `as_str_unchecked()`
5. C type name for header: `"const KrunStr*"` + `"uintptr_t"` (two entries)
6. Generalizing the same pattern to `&[&[u8]]` / `&[&Path]` can wait; libkrun 2.0 only needs `&[&str]`

**Header generation**: `build_header_line` needs to handle "multi-param" types that expand to two C parameters from one Rust parameter.

**C usage**:
```c
KrunStr args[] = { KRUN_STR("/bin/sh"), KRUN_STR("-c"), KRUN_STR("echo hello") };
CHECK(krun_libkruninitbuilder_set_exec(init_builder, KRUN_STR("/bin/sh"), args, 3));
```

**Files**: `ffier-macros/src/lib.rs` (classify_ref_type, ffi_param_tokens, param_conversion, param_c_type_expr, build_header_line), `ffier/src/lib.rs` (FfierBytes already exists, no change needed there).

---

## 3. Support `BorrowedFd<'a>` as a parameter type

**Current**: ffier handles `OwnedFd` via `FfiType` (into_c/from_c converts to/from raw fd).
**Needed**: `BorrowedFd<'a>` → `int` in C, with the lifetime tying the fd to the struct's lifetime.

```rust
pub fn add_tty_port(&mut self, fd: BorrowedFd<'a>) -> Result<u32, KrunError>;
```
```c
KrunError krun_consoledevicebuilder_add_tty_port(KrunConsoleDeviceBuilder b, int fd, uint32_t* result);
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

## 4. Builder Support for Incrementally Configured Types

Types with limited configuration can keep simple constructors like `new()` or `new(args...)`.

libkrun v2.0 only needs builder support for APIs that are configured incrementally. The concrete 2.0 cases are:

- `virtio-console`, where ports are added one by one before producing the final device
- `LibkrunInit`, where exec/env/workdir are configured before producing the payload

The core pattern is already close to working today: use `#[ffier::exportable]` on both the builder type and the final device type, and let `build(self)` consume the builder handle and return the device handle.

Example:

```rust
#[ffier::exportable(prefix = "krun")]
impl<'a> ConsoleDeviceBuilder<'a> {
    pub fn add_tty_port(&mut self, fd: BorrowedFd<'a>) -> Result<u32, KrunError>;
    pub fn add_tty_port_named(&mut self, name: &'a str, fd: BorrowedFd<'a>) -> Result<u32, KrunError>;
    pub fn build(self) -> Result<ConsoleDevice<'a>, KrunError>;
}

#[ffier::exportable(prefix = "krun")]
impl<'a> ConsoleDevice<'a> {
    pub fn builder() -> ConsoleDeviceBuilder<'a>;
}
```

`LibkrunInit` uses the same pattern:

```rust
#[ffier::exportable(prefix = "krun")]
impl<'a> LibkrunInit<'a> {
    pub fn builder(rootfs: &'a mut FsDevice<'a>) -> LibkrunInitBuilder<'a>;
}

#[ffier::exportable(prefix = "krun")]
impl<'a> LibkrunInitBuilder<'a> {
    pub fn set_exec(&mut self, path: &str, args: &[&str]) -> Result<(), KrunError>;
    pub fn build(self) -> Result<LibkrunInit<'a>, KrunError>;
}
```

So item 4 is not only about zero-argument `builder()` constructors. It also covers builder entry points that borrow an already-configured exported object, like `LibkrunInit::builder(&mut rootfs)`.

This assumes one exported impl block per type. Supporting multiple exported impl blocks on the same type would be a separate macro convenience, not a libkrun 2.0 requirement.

---

## 5. No C-Implemented Devices in Initial 2.0

Vtable-based devices implemented through the C ABI are explicitly out of scope for the initial libkrun 2.0 API. That means no `#[ffier::implementable]` work is required to unblock this release.

Possible future work for a later release (for example libkrun 2.1):

- Vtable methods with `BorrowedFd` params
- Vtable methods returning `Result`
- Better supertrait syntax

---

## 6. Remove Auto-Generated `_create()` for Types with `new()`

**Current**: ffier auto-generates `krun_foo_create()` calling `Default::default()` for types without lifetime params.
**Status**: Already fixed in recent commits — auto `create()` was removed. Static methods returning `Self` serve as constructors. ✓

No further changes needed.

---

## 7. `SndDevice` Removal

The `virtio-snd` device will be removed in libkrun v2.0 (pipewire backend was experimental). No ffier changes needed — just don't export it.

---

## 8. Separate C Types for Str/Path (`const char*`) vs Bytes (`const uint8_t*`)

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
| 1 | Drop `Handle` suffix | Small | Yes — required by the documented 2.0 C API |
| 2 | `&[&str]` support | Medium | Yes — needed for `set_exec`, `set_env`, `set_port_map` |
| 3 | `BorrowedFd<'a>` support | Small | Yes — needed for console/serial/input devices |
| 4 | Console/init builder patterns | None to Small | Yes — needed for `virtio-console` and `LibkrunInit` |
| 6 | No auto create() | None | Already fixed ✓ |
| 7 | `SndDevice` removal | None | Just don't export it |
| 8 | Str/Path vs Bytes split | None | Done ✓ |

Out of scope for libkrun 2.0: `#[ffier::implementable]` / vtable-device improvements.
