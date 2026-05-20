//! JSON schema for ffier library metadata.
//!
//! This is the universal binding description format. A program in any language
//! can read this JSON and generate complete bindings to the Rust library —
//! including doc comments, lifetime relationships, C type names, and error
//! variant codes.
//!
//! The schema has two layers per type/param/return:
//! - **Rust-level**: original names, lifetimes, borrow semantics (for Rust/Swift/etc.)
//! - **C-level**: resolved C type names ready for direct use in headers (for C/Python/Go/etc.)

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Top-level library
// ---------------------------------------------------------------------------

/// Complete description of an ffier library.
#[derive(Debug, Serialize, Deserialize)]
pub struct Library {
    /// FFI prefix (e.g. "ft" → functions are `ft_widget_new`, C types are `FtWidget`).
    pub prefix: String,
    pub types: Vec<ExportedType>,
    pub errors: Vec<ErrorType>,
    pub traits: Vec<ImplementableTrait>,
    pub trait_impls: Vec<TraitImpl>,
}

// ---------------------------------------------------------------------------
// Exported types (structs with #[exportable] methods)
// ---------------------------------------------------------------------------

/// A struct exported via `#[ffier::exportable]`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportedType {
    /// Rust struct name (e.g. "Widget").
    pub name: String,
    /// C handle type name (e.g. "FtWidget").
    pub c_name: String,
    /// Stable type tag assigned in `library_definition!`.
    pub type_tag: u32,
    /// Struct-level lifetime params with original names (e.g. `["a"]` for `View<'a>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifetimes: Vec<String>,
    /// Whether this type uses by-value self (builder pattern). When true,
    /// `&mut self` methods in C take a pointer-to-handle (`FtConfig*`) so the
    /// bridge can update the handle in place.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_builder_type: bool,
    pub methods: Vec<Method>,
}

// ---------------------------------------------------------------------------
// Methods — unified for exportable, implementable, and trait_impl
// ---------------------------------------------------------------------------

/// A method. Used in exported types, implementable traits, and trait impls.
#[derive(Debug, Serialize, Deserialize)]
pub struct Method {
    /// Rust method name (e.g. "get_count").
    pub name: String,
    /// Doc comment lines, verbatim. Each entry is one `///` line.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc: Vec<String>,
    pub receiver: Receiver,
    /// Method-level lifetime params with original names (e.g. `["a"]` for `fn foo<'a>(...)`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub method_lifetimes: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Return,
    /// Context-specific fields depending on where this method appears.
    #[serde(flatten)]
    pub context: MethodContext,
}

/// Context-specific fields for a method.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method_context", rename_all = "snake_case")]
pub enum MethodContext {
    /// Method from an `#[exportable]` impl block.
    Exportable {
        /// C FFI function name (e.g. "ft_widget_get_count").
        ffi_name: String,
        /// Whether this is a builder method (returns Self — void at C level,
        /// the caller already has the handle).
        #[serde(default, skip_serializing_if = "is_false")]
        is_builder: bool,
    },
    /// Method from an `#[implementable]` trait or `#[trait_impl]` impl.
    Trait {
        /// Vtable slot index.
        index: usize,
        /// Whether this method has a default impl in the trait.
        #[serde(default, skip_serializing_if = "is_false")]
        has_default: bool,
        /// Raw handle method — receives the raw handle pointer instead of `&self`.
        #[serde(default, skip_serializing_if = "is_false")]
        raw_handle: bool,
    },
}

/// How the method receives `self`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Receiver {
    /// No receiver — static method / associated function.
    None,
    /// `&self`
    Ref,
    /// `&mut self`
    Mut,
    /// `self` (consuming, by value)
    Value,
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// A method parameter with both Rust and C type information.
#[derive(Debug, Serialize, Deserialize)]
pub struct Param {
    /// Parameter name as written in the source (e.g. "name", "source", "s").
    pub name: String,
    /// Rust type as written in the original source, with `Self` replaced by
    /// the concrete struct name, lifetimes preserved.
    /// E.g. `"&'a Widget"`, `"&str"`, `"i32"`, `"BorrowedFd<'_>"`.
    pub rust_type: String,
    /// C type name (e.g. `"int32_t"`, `"FtStr"`, `"FtWidget"`, `"void*"`).
    pub c_type: String,
    /// How this parameter is passed across the FFI boundary.
    pub kind: ParamKind,
}

/// How a parameter is passed across the FFI boundary.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamKind {
    /// Normal parameter — single C value.
    Regular,
    /// `&[&str]` — expands to two C params: `const {Prefix}Str* {name}` + `size_t {name}_len`.
    StrSlice,
    /// `impl Trait` parameter — opaque handle pointer (`void*`).
    ImplTrait {
        /// Trait name (e.g. "Fruit", "Processor").
        trait_name: String,
        /// Dispatch mode: "auto", "concrete", or "vtable".
        dispatch: String,
    },
}

// ---------------------------------------------------------------------------
// Return types
// ---------------------------------------------------------------------------

/// Return type with both Rust and C information.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Return {
    /// Returns nothing (`()`). C: `void`.
    Void,
    /// Returns a single value. C: the value's C type.
    Value {
        /// Original Rust return type (e.g. `"i32"`, `"&str"`, `"Widget"`).
        rust_type: String,
        /// C type name (e.g. `"int32_t"`, `"FtStr"`, `"FtWidget"`).
        c_type: String,
    },
    /// Returns `Result<T, E>`. C: depends on whether T is void, a primitive, or a handle.
    Result {
        /// The Ok type. `None` when `Result<(), E>`.
        ok: Option<ReturnValue>,
        /// Error type Rust name (e.g. "TestError").
        err_type: String,
    },
}

/// The Ok value of a Result return.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReturnValue {
    /// Rust type of the Ok value (e.g. `"i32"`, `"Widget"`).
    pub rust_type: String,
    /// C type name (e.g. `"int32_t"`, `"FtWidget"`).
    pub c_type: String,
    /// Whether this is a handle type. Handle results use null-on-error
    /// convention (return the handle directly, NULL means error).
    /// Non-handle results use out-param convention (result via pointer param).
    pub is_handle: bool,
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// An error enum exported via `#[derive(FfiError)]`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorType {
    /// Rust enum name (e.g. "TestError").
    pub name: String,
    /// C handle type name (e.g. "FtTestError").
    pub c_name: String,
    /// Stable type tag.
    pub type_tag: u32,
    pub variants: Vec<ErrorVariant>,
}

/// A variant of an error enum.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorVariant {
    /// Variant name (e.g. "NotFound").
    pub name: String,
    /// C constant name (e.g. "FT_ERROR_TEST_NOT_FOUND").
    pub c_name: String,
    /// Numeric error code from `#[ffier(code = N)]`.
    pub code: u32,
    /// Human-readable message (from `#[error("...")]` or format string).
    pub message: String,
}

// ---------------------------------------------------------------------------
// Implementable traits
// ---------------------------------------------------------------------------

/// A trait exported via `#[ffier::implementable]`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementableTrait {
    /// Trait name (e.g. "Fruit", "Processor").
    pub name: String,
    /// C handle type name for the trait (e.g. "FtFruit").
    /// Used as the parameter type for `impl Trait` params in C declarations.
    pub c_name: String,
    /// Stable type tag for the vtable wrapper (e.g. "VtableFruit").
    pub type_tag: u32,
    pub methods: Vec<Method>,
    /// Number of methods that belong to this trait (not supertrait methods).
    /// The first `own_method_count` entries in `methods` are this trait's own.
    pub own_method_count: usize,
    /// Highest vtable slot index (including reserved/retired slots).
    pub max_vtable_slot: usize,
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

/// An `impl Trait for Struct` exported via `#[ffier::trait_impl]`.
#[derive(Debug, Serialize, Deserialize)]
pub struct TraitImpl {
    /// Trait name (e.g. "Fruit").
    pub trait_name: String,
    /// Struct name (e.g. "Apple").
    pub struct_name: String,
    /// Lifetime params on the impl block (e.g. `["a"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifetimes: Vec<String>,
    /// Lifetime args on the trait (e.g. `["static"]` or `["a"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trait_lifetime_args: Vec<String>,
    /// Lifetime args on the struct (e.g. `["a"]` or `[]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub struct_lifetime_args: Vec<String>,
    /// Methods provided in this impl.
    pub methods: Vec<Method>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_false(v: &bool) -> bool {
    !v
}

impl Library {
    /// Serialize to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("failed to serialize library metadata")
    }

    /// Deserialize from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}
