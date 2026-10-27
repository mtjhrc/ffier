# Thread safety

Generated opaque handles default to `!Send` and `!Sync`. Declare guarantees on
exported type's inherent impl:

```rust
pub struct Counter(u64);

#[ffier::export(send, sync)]
impl Counter {
    pub fn value(&self) -> u64 {
        self.0
    }
}
```

Use `send`, `sync`, or both independently. Rust checks each declaration against
native type.

- `send`: move owned handle between threads. Destruction may run on receiving
  thread.
- `sync`: share references to handle between threads. Mutable access still
  requires exclusive borrow.

Lifetime bounds still apply. Neither guarantee implies `'static`.

Exported callback traits use ordinary Rust supertraits:

```rust
#[ffier::export]
pub trait Backend: Send + Sync {
    #[ffier(index = 0)]
    fn poll(&mut self) -> i32;
}
```

Declare callback thread guarantees on trait. Explicit use-site bounds such as
`impl Backend + Send` or `impl Backend + Sync` are unsupported.

Rust requires each `Backend` implementation to meet these supertraits. Export
concrete implementor with matching `send`/`sync` flags when its generated handle
also implements `Backend`.

C callbacks must uphold exported trait's guarantees for callback data, vtable,
and callback functions. Keep vtable valid and immutable during use. For `Send`
traits, callback destruction may run on receiving thread. See
[handle ownership](handle-metadata.md#c-side) for allocation and ownership rules.
