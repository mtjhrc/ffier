# Value struct returns

A `#[repr(C)]` struct registered with `value` crosses the ABI by value. It can
also be returned as `Option<T>` or as the success value of a `Result`, which
needs an out-parameter plus a presence signal: `None` has to stay
distinguishable from an output the callee never wrote.

All four shapes are generated from the Rust signature alone, so no annotation
chooses the convention.

## Return shapes

| Rust | C |
| --- | --- |
| `T` | `T` |
| `Result<T, E>` | `MylibResult`, `T* result`, `MylibError* err_out` |
| `Option<T>` | `bool`, `T* result` |
| `Result<Option<T>, E>` | `MylibResult`, `bool* result_is_some`, `T* result`, `MylibError* err_out` |

`T` is the generated C struct type, such as `MylibPacket`. `MylibResult` packs
the library tag and error code; `MylibError` is the opaque error handle.

## By value

```rust
#[ffier::export]
pub fn make_packet(sequence: u32) -> Packet {
    Packet { kind: 1, sequence }
}
```

```c
MylibPacket mylib_make_packet(uint32_t sequence);
```

## Result

The status is the return value; the packet comes back through an out-parameter,
written only on success.

```rust
#[ffier::export]
pub fn checked_packet(sequence: u32) -> Result<Packet, PacketError> {
    if sequence == 0 {
        return Err(PacketError::Bad);
    }
    Ok(Packet { kind: 1, sequence })
}
```

```c
MylibResult mylib_checked_packet(uint32_t sequence,
                                 MylibPacket* result,
                                 MylibError* err_out);
```

## Option

There is no error channel, so the presence flag is the return value itself.

```rust
#[ffier::export]
pub fn maybe_packet(sequence: u32) -> Option<Packet> {
    (sequence > 0).then(|| Packet { kind: 1, sequence })
}
```

```c
bool mylib_maybe_packet(uint32_t sequence, MylibPacket* result);
```

## Result of Option

Presence and failure are separate signals, so both need their own output. This
is a method, so the handle comes first:

```rust
#[ffier::export]
impl Device {
    pub fn next_packet(&mut self) -> Result<Option<Packet>, PacketError> {
        match self.state {
            State::Empty => Ok(None),
            State::Ready(packet) => Ok(Some(packet)),
            State::Broken => Err(PacketError::Bad),
        }
    }
}
```

```c
MylibResult mylib_device_next_packet(MylibDevice handle,
                                     bool* result_is_some,
                                     MylibPacket* result,
                                     MylibError* err_out);
```

The same shape appears in trait vtable slots:

```c
typedef struct {
    uint16_t _num_slots;
    uint16_t _reserved1;
    uint32_t _reserved2;
    void (*drop)(void* self_data);
    MylibResult (*next_packet)(void* self_data, bool* result_is_some,
                               MylibPacket* result, MylibError* err_out);
} MylibSourceVtable;
```

## Borrowed inputs

`&T` and `&mut T` are plain pointers, and `Option<&T>` / `Option<&mut T>` are
nullable ones. The C spelling is identical because C has no separate optional
pointer:

| Rust | C |
| --- | --- |
| `&T` | `const T*` |
| `&mut T` | `T*` |
| `Option<&T>` | `const T*` (null means `None`) |
| `Option<&mut T>` | `T*` (null means `None`) |

```rust
#[ffier::export]
pub fn bump_payload(sample: Option<&mut Packet>) {
    if let Some(sample) = sample {
        sample.sequence += 1;
    }
}
```

```c
void mylib_bump_payload(MylibPacket* sample); /* NULL is fine */
```

Optional value *parameters* by value (`Option<T>`) and value-struct slices are
not supported.

## Caller contract

The value out-parameter is written only when a value is present. Read the
signals in this order:

1. For a `Result` form, check the status. A non-success status means the value
   and presence outputs are untouched; read `err_out` and stop.
2. For an optional form, check the presence flag. Absent means `*result` was
   left untouched and still holds whatever the caller put there.
3. Only then read the value.

In the `Result` forms `err_out` may be null when the caller does not want the
error handle. The bridge never writes through a null `err_out`.

Leaving success outputs untouched means a caller can pass a sentinel-filled
buffer and distinguish "not written" from a real zero value. It does not mean
the buffer is read; never rely on a stale value being meaningful.
