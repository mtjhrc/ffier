# Handle metadata bits

Every handle starts with the same two `u32` fields, then a payload:

```rust
#[repr(C)]
pub struct FfierHandle<T> {
    pub type_tag: u32,
    pub metadata: u32,
    pub value: T,
}

#[repr(C)]
pub struct FfierBorrowedHandle {
    pub type_tag: u32,
    pub metadata: u32,
    pub ptr: *const c_void,
}
```

Bits 0 through 2 are independent flags. Bit 3 introduces a flag that also
reserves a fixed-width field above it: it marks a default-dispatch handle and
makes bits 4 through 19 hold a `u16` method index. The index field is meaningful
only while bit 3 is set, so no reader may treat those bits as an index without
checking the flag first.

## Bits

| Bit | Constant | Question it answers |
| --- | --- | --- |
| 0 | `METADATA_FFIER_ALLOCATED` | Did ffier allocate the handle? |
| 1 | `METADATA_INDIRECT` | Is the value behind a pointer, or inline? |
| 2 | `METADATA_DROP_VALUE` | Is the value's destructor meaningful? |
| 3 | `METADATA_DEFAULT_DISPATCH` | Is this a default-method re-entry handle? Reserves bits 4 through 19 for the index. |
| 4..19 | method index (`u16`) | Which trait method? Only defined while bit 3 is set. |
| 20..31 | unused | — |

### `METADATA_FFIER_ALLOCATED`

Does ffier own the handle allocation?

Set: ffier allocated it, so ffier may free it.

Clear: the allocation belongs to someone else. Two shapes reach this state:

- A caller-owned handle. C builds these with `PREFIX_TEMP_OBJ`
  (`metadata == 0`), typically as a local, or allocates its own storage.
- A `FfierObjectArray` element, where the array owns the allocation.

This bit says nothing about the value inside the payload; the next two bits do.

The two bits are not the same fact. `ffier_handle_new_borrowed` sets this bit
and `METADATA_INDIRECT` while leaving `METADATA_DROP_VALUE` clear, because the
shell is ffier's allocation but the value behind it belongs to someone else.
They stay separate because they answer different questions: `FFIER_ALLOCATED`
is about the handle allocation, `DROP_VALUE` about whether a destructor runs.

### `METADATA_INDIRECT`

Where does the value live?

Set: the payload field is a `*const c_void` to the real `T`. Both supported
constructors pair this with a pointee owned by someone else:
`ffier_handle_new_borrowed` stores a borrow for `&T` and `&mut T` returns, and
object array elements borrow from the array's owner.

Clear: the payload is an inline `T`, which the handle owns whenever it is
ffier-allocated.

The bits record layout and allocation, not who owns the pointee. `T` and the
type tag do not disambiguate either: the same pair describes an owned inline
handle and a borrowed indirect one. That is why an indirect handle owning its
own pointee is unsupported — `ffier_handle_drop` would free only the handle and
leak. Supporting it needs an owner bit, a distinct type tag, or a stored
destroy callback.

### `METADATA_DROP_VALUE`

Does dropping the handle also destroy the value?

Set: the value's destructor is meaningful. For an inline payload this is the
ordinary owned handle. For a vtable payload the destructor is the generated
wrapper's `Drop`, which calls the vtable `drop` slot so the caller can release
`user_data`.

Clear: the value belongs to someone else — a borrowed shell pointing into a
parent, or an object-array element. The handle may still be freed; the value
must not be dropped.

This bit is separate from layout on purpose. `INDIRECT` says where the value
lives; this bit says whether it is ours to destroy. Keeping them apart is what
makes "free the handle" and "drop the value" two decisions instead of one
inferred from `INDIRECT`.

An indirect handle that sets this bit describes a pointee ffier owns. No
producer creates one yet, and `ffier_handle_drop` asserts rather than leaking.
Supporting it means dropping through the pointer before freeing the handle.

### `METADATA_DEFAULT_DISPATCH`

Set: the handle is a temporary built by generated client bindings to re-enter
the library's default method implementation. While this bit is set, bits 4
through 19 hold the method index as a `u16`. The field starts at bit 4
(`METADATA_DEFAULT_DISPATCH_SHIFT`), is covered by
`METADATA_DEFAULT_DISPATCH_METHOD_MASK`, and is decoded by
`default_dispatch_method_index`. Its width matches the vtable header's `u16`
slot count, so no vtable index can overflow the field. When the bit is clear
those bits carry no meaning and are available for future flags.

Such handles carry no allocation bit, so they can never be consumed or freed.

## States

Each row is a combination the tree produces today.

| State | `FFIER_ALLOCATED` | `INDIRECT` | `DROP_VALUE` |
| --- | --- | --- | --- |
| Inline owned handle | 1 | 0 | 1 |
| Borrowed shell (`&T` return) | 1 | 1 | 0 |
| `FfierObjectArray` element | 0 | 1 | 0 |
| Caller-owned inline (`PREFIX_TEMP_OBJ`) | 0 | 0 | 0 |

`INDIRECT` without `FFIER_ALLOCATED` is exactly an array element, which is how
`ffier_handle_drop` recognizes one.

`INDIRECT` with `DROP_VALUE` is deliberately absent: that pair would mean ffier
owns a pointee reached through a pointer, which nothing constructs yet.
`ffier_handle_drop` asserts on it rather than freeing the handle and leaking the
pointee.

## How the runtime uses the bits

`ffier_handle_borrow` / `ffier_handle_borrow_mut` read `INDIRECT` only. They
either follow the stored pointer or read the inline value. They never free.

`ffier_handle_consume` requires inline and ffier-allocated. It asserts on
`INDIRECT` first (the value is owned elsewhere), then on the allocation bit
(ffier cannot free foreign memory). Consuming anything else would be unsound,
so these are `assert`, not `debug_assert`: the input crosses the FFI boundary
from caller code, where release builds are exactly when corruption would
happen.

`ffier_handle_drop` asserts the allocation bit: only a handle ffier allocated
may be destroyed, and freeing a caller-owned one or a single array element is a
contract violation. It then reads `DROP_VALUE` to decide whether the destructor
runs, and the payload shape decides how the allocation itself is freed. The two
steps are independent — that separation is the point of the bit.

## Invariants

- Each bit answers one question: allocation, layout, value destruction. No bit
  changes meaning depending on another.
- `DROP_VALUE` is cleared whenever the value belongs to someone else, which is
  every indirect handle produced today. Setting it with `INDIRECT` is rejected
  by `ffier_handle_drop`, because dropping through the pointer is not
  implemented.
- Bits 4 through 19 hold the `u16` method index only while
  `METADATA_DEFAULT_DISPATCH` is set. Without that flag they are reserved for
  future flags, and bits 20 and above are always free.
- A vtable index must leave room for the slot count to fit in `u16`, because
  the vtable header stores that count as `u16`. The slot count is the highest
  index plus one, so `#[ffier(index = N)]` accepts at most 65534 and rejects
  65535 onward. `reserved(N)` follows the same rule.
- A handle carrying no bits is caller-owned and inline. ffier only reads it.

## C side

C sees the field only as an opaque `uint32_t metadata` inside the generated
handle struct. Two construction paths exist:

- `PREFIX_TEMP_OBJ(type_tag, &vtable, user_data)` builds a caller-owned handle
  with `metadata == 0`, valid for borrowed parameters only.
- `PREFIX_vtable_handle_new(type_tag, &vtable, user_data)` returns a handle
  allocated by that library, so `FFIER_ALLOCATED` is set and the same library
  frees it. Use it for owned parameters.
