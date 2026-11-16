# Never returns

Use `#[ffier(never_type = Never)]` for an operation that never returns a success value.
This can currently be used as a direct return type or as the success type of
`Result<T, E>`.

Define the alias to match `ffier::Never`, using the `rustversion` dependency:

```rust
#[rustversion::since(1.100)]
pub type Never = !;
#[rustversion::before(1.100)]
pub type Never = ();

#[cfg_attr(feature = "ffi", ffier::export)]
pub trait EventLoop {
    #[cfg_attr(feature = "ffi", ffier(index = 0, never_type = Never))]
    fn run(&mut self) -> Result<Never, EventLoopError>;
}
```

`EventLoop::run` returns only on error; it never returns `Ok`.
On Rust 1.100 and later, `Never` is `!`: **Rust enforces this at compile time**,
rejecting success values and trait implementations that substitute
`Result<(), E>`. On older compilers, `Never` is `()`, and the bridge and
callback wrappers abort if an implementation returns success.

The generated Rust client uses `Result<ffier::Never, EventLoopError>`, with the
same compiler-dependent meaning. C callers pass the event loop handle and an
error output. The call borrows the handle, matching Rust's `&mut self`:

```c
FtError err = NULL;
FtResult result = ft_event_loop_run(event_loop, &err);
/* Only an error can return here. */
assert(result != FT_RESULT_SUCCESS);
ft_error_destroy(err);
```

Pass `NULL` for the error output to discard the error object. Generated clients
and native callers of C callbacks abort on an unexpected normal return or
success result from the other side.

Apply the annotation to each exported function or method with this contract,
including trait definitions and exported implementations. Literal `-> !` and
`Result<!, E>` returns are recognized without an annotation. A direct never
return becomes `!` in the Rust client and `void` in C.
