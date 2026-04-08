# Trait reference dispatch: design analysis

## The core question

When a user type (client-side `Banana`) is passed as `&impl Fruit` across FFI, what exactly is the library storing, and is it safe?

## How the bridge handles `&impl Trait`

The bridge receives `*mut c_void` (the handle) and dispatches by `type_id`. For a user type wrapped in `FfierTaggedRef`:

```
FfierTaggedRef (on caller's stack):
┌──────────────┐
│ type_id      │  = TypeId::of::<VtableFruit>()
│ user_data    │  → points to Banana on user's stack (valid while user owns it)
│ vtable       │  → points to static VtableStruct (valid forever)
└──────────────┘
```

The bridge casts the handle to `&FfierTaggedBox<VtableFruit>`. Due to matching layout, this gives `&VtableFruit` which is `{ user_data, vtable }` — these two fields live INSIDE the `FfierTaggedRef`.

### The subtle problem

The individual fields are fine:
- `user_data` → Banana on user's stack. Valid as long as user keeps Banana alive. ✓
- `vtable` → static memory. Valid forever. ✓

But the `&VtableFruit` reference ITSELF points into the `FfierTaggedRef` struct — which is on the wrapper function's stack. When the wrapper returns, the `FfierTaggedRef` is gone.

The library receives `&VtableFruit` as `&impl Fruit`. If the method is call-scoped (just reads the value and returns), this is fine — the `FfierTaggedRef` is alive for the call duration.

But if the library STORES the reference (like `View<'a>` stores `&'a Widget`):

```rust
struct ViewLike<'a> {
    source: &'a VtableFruit,  // points into FfierTaggedRef on stack
}
```

After the wrapper returns, `source` is dangling. Not because the Banana died — but because the `{ user_data, vtable }` pair that `source` points to was on the stack.

### Why known types don't have this problem

For `Apple`, the `FfierTaggedBox<Apple>` is heap-allocated when `Apple::new()` is called. The wrapper IS the value — same heap object. `&Apple` points into a stable heap allocation that lives as long as the handle exists. The user controls the lifetime.

For user types, the wrapper (`FfierTaggedRef`) and the value (`Banana`) are SEPARATE. The value is stable (user owns it). The wrapper is temporary (on the stack).

## The fix: heap-allocate the wrapper

If we heap-allocate the `FfierTaggedRef`, the `&VtableFruit` reference points to stable heap memory. The user needs to keep the allocation alive for the borrow duration.

For call-scoped borrows: the wrapper function's `FfierHandleRef` binding holds the allocation, drops after the call. Automatic, correct.

For stored borrows: the user needs to manage the `FfierHandleRef` explicitly — keeping it alive as long as the library holds the reference. This changes the API for stored-reference methods.

### Cost

One small heap allocation (~24 bytes: type_id + 2 pointers) per user-type ref param. Known types: zero cost (existing handle pointer).

## Summary table

| Scenario | Stack wrapper | Heap wrapper |
|----------|--------------|-------------|
| Call-scoped borrow | ✅ works | ✅ works |
| Stored borrow | ❌ dangling | ✅ works (if user manages lifetime) |
| Known types | N/A (use existing handle) | N/A |
| Allocation cost | Zero | ~24 bytes |

## Interaction with `&dyn Trait`

Same analysis applies. The `&dyn Trait` case adds one wrinkle: reconstructing `&dyn Trait` from `user_data` in the trampolines requires either:
1. A `transmute` assuming fat pointer layout is `(data, vtable)` — works on all Rust targets but not officially guaranteed
2. A `Box<&dyn Trait>` indirection — one extra pointer dereference, no layout assumption
3. Storing the fat pointer in a struct field — can't use `#[repr(C)]` because `&dyn Trait` size isn't guaranteed

Option 1 is pragmatic (every Rust FFI crate relies on this). Option 2 adds one more small allocation.

## Current implementation

- Known types by ref: ✅ `FfierHandleRef::Handle(self.0)` — zero cost
- User types by ref via `&impl Trait`: ⚠️ `FfierHandleRef::Tagged(FfierTaggedRef)` — call-scoped only (stack wrapper)
- User types by ref via `&dyn Trait`: ⚠️ `__as_handle_dyn()` default — call-scoped only, relies on fat pointer layout
- By-value dispatch: ✅ fully working for all types
- `FfierBoxDyn<dyn Trait>`: ✅ for vtable dispatch fallback
- Dispatch limit: ✅ 64 branches, auto hybrid, per-param `#[ffier(dispatch = concrete|vtable)]`
