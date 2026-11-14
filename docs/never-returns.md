# Never returns

FFier recognizes `!` and the reserved type name `Never` in return signatures.
They can appear as the whole return type or as the success type of
`Result<T, E>`. Use `!` directly where Rust supports it. Define `Never` as a
type alias for `()` or `!`. FFier checks the alias when it appears in a return
signature.

```rust
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn terminate(status: i32) -> ! {
    std::process::exit(status)
}

type Never = ();

#[cfg_attr(feature = "ffi", ffier::export)]
pub trait EventLoop {
    #[cfg_attr(feature = "ffi", ffier(index = 0))]
    fn run(&mut self) -> Result<Never, EventLoopError>;
}
```

The bridge aborts if a library returns a `Never` value or an `Ok` result.
Generated Rust clients return `!` or `Result<core::convert::Infallible, E>`.

In C, a direct never return uses `void`. A fallible never return uses a packed
error code and an error output pointer. Pass `NULL` to discard the error object.
Generated Rust clients and native callers of C callbacks abort if the other
side returns from a direct never call or reports success from a fallible call.
