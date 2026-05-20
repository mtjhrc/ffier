//! JSON schema for ffier library metadata.
//!
//! This is the contract between the bridge proc-macro (which serializes) and
//! code generators (which deserialize). Every field that affects the generated
//! API surface must be represented here — param names, lifetime names, doc
//! comments, error variants, trait methods, etc.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Top-level library
// ---------------------------------------------------------------------------

/// Complete description of an ffier library.
#[derive(Debug, Serialize, Deserialize)]
pub struct Library {
    /// FFI prefix (e.g. "ft" → functions are `ft_widget_new`, types are `FtWidget`).
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
    /// Stable type tag assigned in `library_definition!`.
    pub type_tag: u32,
    /// Struct-level lifetime params with original names (e.g. `["a"]` for `View<'a>`).
    pub lifetimes: Vec<String>,
    pub methods: Vec<Method>,
}

// ---------------------------------------------------------------------------
// Methods
// ---------------------------------------------------------------------------

/// A method on an exported type, implementable trait, or trait impl.
#[derive(Debug, Serialize, Deserialize)]
pub struct Method {
    /// Rust method name (e.g. "get_count").
    pub name: String,
    /// C FFI function name suffix (e.g. "widget_get_count"). Empty for trait methods.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ffi_name: String,
    /// Doc comment lines, verbatim. Each entry is one `///` line.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc: Vec<String>,
    pub receiver: Receiver,
    /// Method-level lifetime params with original names (e.g. `["a"]` for `fn foo<'a>(...)`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub method_lifetimes: Vec<String>,
    pub params: Vec<Param>,
    /// Return type as written in the original Rust source (e.g. `"Result<i32, TestError>"`).
    /// "()" for void.
    pub rust_ret: String,
    /// Whether this is a builder method (returns Self, which is invisible at
    /// the FFI boundary — the caller already has the handle).
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_builder: bool,
    /// C bridge return type name. Determines what the extern "C" fn returns.
    /// Not used by Rust client codegen (which uses `rust_ret` instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c_ret_type: Option<String>,
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

/// A method parameter.
#[derive(Debug, Serialize, Deserialize)]
pub struct Param {
    /// Parameter name as written in the source (e.g. "name", "source", "s").
    pub name: String,
    /// Rust type as written in the original source, with `Self` replaced by
    /// the concrete struct name but lifetimes preserved.
    /// E.g. `"&'a Widget"`, `"&str"`, `"i32"`, `"impl Fruit"`, `"BorrowedFd<'_>"`.
    pub rust_type: String,
    /// Param kind. Determines how the bridge and client codegen handle this param.
    pub kind: ParamKind,
}

/// How a parameter is passed across the FFI boundary.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamKind {
    /// Normal parameter — bridge type resolved via `<T as FfiType>::CRepr`.
    Regular,
    /// `&[&str]` — slice of string references, expands to two C params (ptr + len).
    StrSlice,
    /// `impl Trait` parameter — the bridge dispatches to concrete types.
    ImplTrait {
        /// Trait name (e.g. "Fruit", "Processor").
        trait_name: String,
        /// Dispatch mode: "auto", "concrete", or "vtable".
        dispatch: String,
    },
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// An error enum exported via `#[derive(FfiError)]`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorType {
    /// Rust enum name (e.g. "TestError").
    pub name: String,
    /// Stable type tag.
    pub type_tag: u32,
    pub variants: Vec<ErrorVariant>,
}

/// A variant of an error enum.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorVariant {
    /// Variant name (e.g. "NotFound").
    pub name: String,
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
    /// Stable type tag.
    pub type_tag: u32,
    pub methods: Vec<TraitMethod>,
    /// Number of methods that belong to this trait (not supertrait methods).
    /// The first `own_method_count` entries in `methods` are this trait's own.
    pub own_method_count: usize,
    /// Highest vtable slot index (including reserved/retired slots).
    pub max_vtable_slot: usize,
}

/// A method in an implementable trait.
#[derive(Debug, Serialize, Deserialize)]
pub struct TraitMethod {
    /// Method name.
    pub name: String,
    /// Vtable slot index.
    pub index: usize,
    /// Whether this method has a default impl in the trait.
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_default: bool,
    /// Raw handle method — receives `*const FfierHandle<Self>` instead of `&self`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub raw_handle: bool,
    pub receiver: Receiver,
    pub params: Vec<Param>,
    /// Return type as written in Rust source. "()" for void.
    pub rust_ret: String,
    /// Doc comment lines, verbatim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc: Vec<String>,
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
    pub methods: Vec<TraitMethod>,
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
