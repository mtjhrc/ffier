//! Proc-macro entry points for ffier export and bridge generation.

use std::sync::atomic::AtomicUsize;

use proc_macro::TokenStream;

mod alias;
mod attrs;
mod bitflags;
mod bridge;
mod cfg;
mod contracts;
mod error;
mod export;
mod library;
mod meta;
mod reexport;
mod signature;
mod types;
mod vtable;

/// Counter for generating unique `#[macro_export]` macro names.
/// The exported name is an implementation detail — users access the macro
/// only through the `pub use ... as __ffier_meta_*` alias placed next to the type.
static MACRO_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Export a Rust item for FFI consumption.
///
/// Dispatches automatically based on the annotated item:
/// - **`impl Struct { ... }`** — export struct methods as C functions
/// - **`impl Trait for Struct { ... }`** — bridge trait impl methods to C
/// - **`trait Foo { ... }`** — declare a trait implementable from C via vtable
/// - **`enum Foo { ... }`** — export enum variants as C constants
/// - **`fn foo(...) { ... }`** — export a free function to C
///
/// Trait definitions accept optional arguments:
/// ```ignore
/// #[ffier::export(reserved(1, 3), foreign, bless = "error_trait")]
/// trait Foo { ... }
/// ```
///
/// All item kinds also accept `cfg = "predicate"` — see [`ExportCfg`](export::ExportCfg).
/// Inherent impls accept `send` and `sync`, checked against the exported type.
/// Traits use native `Send`/`Sync` supertraits.
/// Every opaque lifetime requires `covariant('a)`, `contravariant('a)`, or
/// `invariant('a)`. Covariance and contravariance are compiler checked.
#[proc_macro_attribute]
pub fn export(attr: TokenStream, item: TokenStream) -> TokenStream {
    export::export(attr, item)
}

/// Define a `bitflags!` type and export it for FFI.
///
/// Wraps the `bitflags!` macro invocation: the type is always defined,
/// and the FFI metadata (FfiType impl, schema entry) is gated behind
/// the specified Cargo feature.
///
/// ```ignore
/// ffier::export_bitflags! {
///     if = "ffi",
///     bitflags::bitflags! {
///         #[derive(Debug, Clone, Copy, PartialEq, Eq)]
///         pub struct Permissions: u32 {
///             const READ  = 0b001;
///             const WRITE = 0b010;
///             const EXEC  = 0b100;
///         }
///     }
/// }
/// ```
#[proc_macro]
pub fn export_bitflags(input: TokenStream) -> TokenStream {
    bitflags::export_bitflags(input)
}

/// Derive `ffier::FfiError` for an error enum.
///
/// Each variant must have `#[ffier(code = N)]` and optionally
/// `#[ffier(code = N, message = "...")]`. Without an explicit message,
/// the static message is the variant name. Fieldless variants may use either
/// `Variant` or `Variant()` syntax; generated clients preserve that choice.
/// Generated fieldless `Display` uses this static message, not native `Display`.
/// Keep both messages consistent to avoid confusing differences across FFI.
///
/// ```ignore
/// #[derive(ffier::FfiError)]
/// #[non_exhaustive]
/// pub enum CalcError {
///     #[ffier(code = 1)]
///     DivisionByZero,
///     #[ffier(code = 2, message = "integer overflow")]
///     Overflow,
/// }
/// ```
#[proc_macro_derive(FfiError, attributes(ffier))]
pub fn derive_ffi_error(input: TokenStream) -> TokenStream {
    error::derive_ffi_error(input)
}

/// Generate owned and mutable borrowed dynamic dispatch forwarding.
///
/// `#[ffier::export]` on traits implies `#[ffier::dispatch]` — use this
/// annotation alone when you want dynamic dispatch fallback without
/// exporting the trait's vtable to C.
#[proc_macro_attribute]
pub fn dispatch(attr: TokenStream, item: TokenStream) -> TokenStream {
    export::dispatch(attr, item)
}

/// Define the list of exported types for a library.
///
/// The `library_tag` (1–255) uniquely identifies this library. It is
/// composed into the upper 8 bits of every type's tag, leaving 24 bits
/// for the per-type tag. This ensures that handles from different
/// ffier libraries are never accidentally interchangeable — a type tag
/// mismatch will trigger a panic with a clear diagnostic.
///
/// Every entry (except `TraitName for StructName`) must have an explicit
/// type tag: `Name = N`. Tags must be nonzero and unique across the library.
/// Entries can be bare paths or qualified paths (e.g. `crate::submod::Foo`).
///
/// ```ignore
/// ffier::library_definition!("mylib", library_tag = 1,
///     CalcError = 1,
///     Calculator = 2,
///     crate::submod::TextBuffer = 3,
///     BufferError = 4,
///     trait Processor = 10,
///     trait crate::traits::Fruit = 11,
///     crate::traits::Fruit for crate::types::Apple,
/// );
///
/// // Same crate:
/// ffier::generate_bridge!(local = __ffier_mylib_metadata,
///     schema_output = "../../target/ffier-mylib.json");
/// // Or from a separate cdylib crate:
/// ffier::generate_bridge!(external = mylib::__ffier_mylib_metadata,
///     schema_output = "../../target/ffier-mylib.json");
/// ```
///
/// ## Shared primitives prefix
///
/// When multiple ffier libraries share the same primitive types (Str, Bytes,
/// Result, VtableHandle), set `primitives_prefix` to a common prefix:
///
/// ```ignore
/// // krun_init uses "krun" primitives → KrunStr, KrunResult, etc.
/// ffier::library_definition!("krun_init", library_tag = 2,
///     primitives_prefix = "krun",
///     InitError = 1,
///     Config = 2,
/// );
/// ```
///
/// The C header wraps primitive type definitions in `#ifndef` guards so
/// whichever header is included first defines them. Functions like
/// `str_free` always use the library prefix (`krun_init_str_free`).
///
/// Supports three entry kinds:
/// - `Path = N` — exported struct or error enum with type tag
/// - `trait Path = N` — exported trait with type tag
/// - `TraitPath for StructPath` — trait impl bridge (uses the struct's tag)
///
/// Each annotated type generates a `__ffier_meta_*` alias macro next to the
/// type via `pub use`. This macro resolves those aliases from the given paths
/// and invokes their `@on_library_export` arm to generate type impls at the crate root.
#[proc_macro]
pub fn library_definition(input: TokenStream) -> TokenStream {
    library::library_definition(input)
}

#[doc(hidden)]
#[proc_macro]
pub fn __generate_vtable(input: TokenStream) -> TokenStream {
    vtable::__generate_vtable(input)
}

/// Automatically re-export ffier metadata alongside a `use` import.
#[proc_macro_attribute]
pub fn reexport(attr: TokenStream, item: TokenStream) -> TokenStream {
    reexport::reexport(attr, item)
}

/// Generate `extern "C"` bridge functions and write JSON schema.
///
/// Called by the `__ffier_chain` base case with accumulated metadata
/// from all registered types. Not intended for direct use.
#[doc(hidden)]
#[proc_macro]
pub fn __generate_bridge(input: TokenStream) -> TokenStream {
    bridge::generate_batch_impl(input.into()).into()
}

/// Generate `extern "C"` bridge functions for an ffier library.
///
/// Takes a mode keyword, the metadata macro path, and the schema output path.
///
/// - `local` — the macro is in the same crate as `library_definition!`.
///   Uses bare names internally to work around rust#52234.
/// - `external` — the macro is in a different crate (the typical cdylib case).
///   Uses `$crate::` paths for cross-crate resolution.
///
/// `schema_output` is the path where the JSON schema is written.
/// Relative paths are resolved from `CARGO_MANIFEST_DIR` (the directory
/// containing the `Cargo.toml` of the crate calling this macro).
///
/// ```ignore
/// // Same crate as library_definition!:
/// ffier::generate_bridge!(local = __ffier_ft_metadata,
///     schema_output = "../../target/ffier-ft.json");
///
/// // Separate cdylib crate:
/// ffier::generate_bridge!(external = mylib::__ffier_ft_metadata,
///     schema_output = "../../target/ffier-ft.json");
/// ```
#[proc_macro]
pub fn generate_bridge(input: TokenStream) -> TokenStream {
    bridge::generate_bridge(input)
}
