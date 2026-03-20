# ffier Changes Needed for libkrun v2.0

Changes needed in the ffier framework (`~/Dev2/ffier/t/main/`) to support the libkrun v2.0 API design.

---

## 1. Drop `Handle` suffix from C type names

**Status: DONE** ✓

`typedef void* KrunVmmBuilder;` — no more `Handle` suffix. Both `exportable` and `implementable` updated. RTTI TYPE_IDs changed (hash of new string). All examples updated.

---

## 2. Support `&[&str]` as a parameter type

**Status: DONE** ✓

`&[&str]` → `const KrunStr* items, uintptr_t items_len`. Detected via `is_str_slice()`, expands to two C params via `ParamKind::StrSlice`. Conversion builds a `Vec<&str>` from the `FfierBytes` array. Tested end-to-end with `set_label()` in the example.

---

## 3. Support `BorrowedFd<'a>` as a parameter type

**Status: DONE** ✓

`FfiType for BorrowedFd<'static>` impl added to `ffier/src/lib.rs`. Maps to `int` in C. The macro erases lifetimes to `'static`, so `BorrowedFd<'a>` in user code matches the impl. Tested end-to-end with `fd_number()` in the example.

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
| 1 | Drop `Handle` suffix | Small | Done ✓ |
| 2 | `&[&str]` support | Medium | Done ✓ |
| 3 | `BorrowedFd<'a>` support | Small | Done ✓ |
| 4 | Console/init builder patterns | None | Already works ✓ (krun example demonstrates borrowed-object builder entry points) |
| 6 | No auto create() | None | Already fixed ✓ |
| 7 | `SndDevice` removal | None | Just don't export it |
| 8 | Str/Path vs Bytes split | None | Done ✓ |

Out of scope for libkrun 2.0: `#[ffier::implementable]` / vtable-device improvements.
