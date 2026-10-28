# Never returns

Use `#[ffier(never_type = Never)]` when a return type represents an operation
that never succeeds normally. This applies both to a direct return and to the
success type of `Result<T, E>`. The annotation names the alias to treat as
uninhabited at the FFI boundary. ffier checks that this alias matches the direct
return or `Result` success type; it does not infer semantics from its name or
read source files.

A unit alias lets the native library keep compiling on stable Rust:

```rust
pub type Never = ();

#[cfg_attr(feature = "ffi", ffier::export)]
impl Vm {
    #[cfg_attr(feature = "ffi", ffier(never_type = Never))]
    pub fn run(mut self) -> Result<Never, VmmError> {
        loop {
            self.process_events()?;
        }
    }
}
```

The alias may have any name and needs no `library_definition!` registration.
It remains an ordinary Rust alias. With the `ffi` feature disabled, both
annotations disappear and no FFI code is generated. The native Rust signature
does not change when the feature is enabled either; the contract is enforced
when the function crosses the FFI boundary.

Place the annotation on each exported function or method whose direct return
or `Result` success type has this meaning, including exported trait methods and
concrete trait implementations. Other `Result<(), E>` methods continue to allow
`Ok(())`.

```rust
#[cfg_attr(feature = "ffi", ffier::export)]
pub trait EventLoop {
    #[cfg_attr(feature = "ffi", ffier(index = 0, never_type = Never))]
    fn run(&mut self) -> Result<Never, VmmError>;
}
```

Literal diverging returns are recognized automatically and require no marker:

```rust
#[cfg_attr(feature = "ffi", ffier::export)]
pub fn terminate(status: i32) -> ! {
    std::process::exit(status)
}
```

Generated Rust clients expose a direct never return as `!` and a fallible never
return as `Result<core::convert::Infallible, E>`. Both work on stable Rust without
using `!` as a generic argument. Callback trait implementations use the same
generated signatures.

At the C boundary, a direct never return has no return value and must not
return. A fallible never return uses the existing packed result and optional
error-handle output, with no success-value output:

```c
FtResult ft_vm_run(FtVm vm, FtError* err_out);
```

Returning a nonzero error result is valid. Returning success is not: a native
implementation that returns from a marked direct return or yields `Ok(())`
aborts at the bridge. Generated clients and callback wrappers also reject an
unexpected normal return or success result. They never manufacture an
uninhabited Rust value.
