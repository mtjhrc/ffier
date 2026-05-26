//! JSON schema for ffier library metadata.
//!
//! This is the universal binding description format. A program in any language
//! can read this JSON and generate complete bindings to the Rust library —
//! including doc comments, lifetime relationships, C type names, and error
//! variant codes.
//!
//! All types are registered in a single `type_registry` keyed by Rust type
//! name. Params and returns reference types by name + usage modifiers
//! (ref kind, lifetimes).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Top-level library
// ---------------------------------------------------------------------------

/// Complete description of an ffier library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    /// FFI prefix (e.g. "ft" → functions are `ft_widget_new`, C types are `FtWidget`).
    pub prefix: String,
    /// All types used in the library, keyed by Rust name.
    /// Includes primitives (`"i32"`), builtins (`"str"`), handles (`"Widget"`),
    /// errors (`"TestError"`), and traits (`"Fruit"`).
    pub type_registry: BTreeMap<String, TypeEntry>,
    pub exported_types: Vec<ExportedType>,
    pub errors: Vec<ErrorType>,
    /// Plain enums exported as C `#define` constants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_constants: Vec<EnumType>,
    /// Free (non-method) functions exported via `#[ffier::exportable]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub free_functions: Vec<FreeFunction>,
    pub traits: Vec<ImplementableTrait>,
    pub trait_impls: Vec<TraitImpl>,
}

// ---------------------------------------------------------------------------
// Type registry
// ---------------------------------------------------------------------------

/// An entry in the type registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeEntry {
    /// What kind of type this is.
    pub kind: TypeKind,
    /// C type name (e.g. `"int32_t"`, `"FtWidget"`, `"FtStr"`).
    pub c_type: String,
    /// Stable type tag from `library_definition!`. Only present for
    /// handles, errors, and implementable traits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_tag: Option<u32>,
    /// Lifetime parameters on the type definition (e.g. `["a"]` for `View<'a>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifetime_params: Vec<String>,
}

/// The kind of a type in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeKind {
    /// Primitive integer or bool: `i32`, `u64`, `bool`, `usize`, etc.
    Primitive,
    /// String slice: `str`. Passed as a struct (ptr+len).
    String,
    /// Byte slice: `[u8]`. Passed as a struct (ptr+len).
    Bytes,
    /// Type alias for another type (e.g. `BorrowedFd` → `i32`).
    Alias {
        /// The underlying type name this aliases.
        alias_of: std::string::String,
        /// Whether this alias transfers ownership (e.g. `OwnedFd` vs `BorrowedFd`).
        owned: bool,
    },
    /// Opaque handle type (a struct exported via `#[exportable]`).
    Handle,
    /// Error type (an enum exported via `#[derive(FfiError)]`).
    Error,
    /// Trait (from `#[implementable]` or discovered via `#[trait_impl]`).
    Trait,
    /// Plain enum with explicit discriminant values and a `#[repr(uN)]`.
    /// At the C ABI level the parameter is the underlying integer type
    /// (recorded in `alias_of`), but the schema carries the variant names
    /// and values so generators can emit `#define` constants / typed enums.
    Enum {
        /// The underlying integer type name (e.g. `"u32"`, `"u64"`).
        alias_of: std::string::String,
    },
    /// Sentinel type for builder methods that return `Self`.
    /// At the C ABI level this is `void` (the new handle is written back
    /// through a double pointer), but Rust/Swift generators can recover
    /// `-> Self` from this.
    ReplacesSelf,
}

// ---------------------------------------------------------------------------
// Type references (used in params, returns)
// ---------------------------------------------------------------------------

/// A reference to a type in the registry, with usage-site modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeRef {
    /// Key into `type_registry` (e.g. `"Widget"`, `"i32"`, `"str"`).
    #[serde(rename = "type")]
    pub type_name: String,
    /// How the type is accessed at this usage site.
    #[serde(default, skip_serializing_if = "is_ref_none")]
    pub ref_kind: RefKind,
    /// Lifetime on the reference itself (e.g. `"a"` in `&'a Widget`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_lifetime: Option<String>,
    /// Lifetime arguments applied to the type's params
    /// (e.g. `["a"]` in `View<'a>`, `["b"]` in `&'a View<'b>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_args: Vec<String>,
}

/// How a type is accessed at a usage site.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// By value.
    #[default]
    None,
    /// `&T` or `&'a T`.
    Shared,
    /// `&mut T` or `&'a mut T`.
    Mut,
}

impl TypeRef {
    /// Reconstruct the Rust type syntax from this reference.
    /// E.g. `TypeRef { type: "Widget", ref_kind: Shared, ref_lifetime: Some("a") }` → `&'a Widget`
    /// E.g. `TypeRef { type: "View", type_args: ["a"] }` → `View<'a>`
    pub fn to_rust_type(&self) -> String {
        let mut s = String::new();

        // Reference prefix
        match self.ref_kind {
            RefKind::None => {}
            RefKind::Shared => {
                s.push('&');
                if let Some(lt) = &self.ref_lifetime {
                    s.push('\'');
                    s.push_str(lt);
                    s.push(' ');
                }
            }
            RefKind::Mut => {
                s.push('&');
                if let Some(lt) = &self.ref_lifetime {
                    s.push('\'');
                    s.push_str(lt);
                    s.push(' ');
                }
                s.push_str("mut ");
            }
        }

        // Type name
        s.push_str(&self.type_name);

        // Generic lifetime args
        if !self.type_args.is_empty() {
            s.push('<');
            for (i, arg) in self.type_args.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push('\'');
                s.push_str(arg);
            }
            s.push('>');
        }

        s
    }

    /// Reconstruct with lifetimes erased to `'static` (for extern declarations).
    pub fn to_rust_type_static(&self) -> String {
        let mut s = String::new();

        match self.ref_kind {
            RefKind::None => {}
            RefKind::Shared => {
                s.push_str("&'static ");
            }
            RefKind::Mut => {
                s.push_str("&'static mut ");
            }
        }

        s.push_str(&self.type_name);

        if !self.type_args.is_empty() {
            s.push('<');
            for (i, _) in self.type_args.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str("'static");
            }
            s.push('>');
        }

        s
    }
}

// ---------------------------------------------------------------------------
// Exported types (structs with #[exportable] methods)
// ---------------------------------------------------------------------------

/// A struct exported via `#[ffier::exportable]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedType {
    /// Rust struct name — key into `type_registry`.
    pub name: String,
    /// FFI destroy function name (e.g. `"ft_widget_destroy"`).
    pub destroy_ffi_name: String,
    /// Whether this type uses by-value self (builder pattern). When true,
    /// by-value self methods in C take a pointer-to-handle so the bridge
    /// can update the handle in place.
    pub is_builder_type: bool,
    pub methods: Vec<Method>,
}

// ---------------------------------------------------------------------------
// Methods — unified for exportable, implementable, and trait_impl
// ---------------------------------------------------------------------------

/// A method. Used in exported types, implementable traits, and trait impls.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method_context", rename_all = "snake_case")]
pub enum MethodContext {
    /// Method from an `#[exportable]` impl block.
    Exportable {
        /// C FFI function name (e.g. "ft_widget_get_count").
        ffi_name: String,
    },
    /// Method from an `#[implementable]` trait or `#[trait_impl]` impl.
    Trait {
        /// C FFI function name (e.g. `"ft_fruit_eat"` for dispatch,
        /// `"ft_apple_eat"` for a trait impl).
        ffi_name: String,
        /// Vtable slot index.
        index: usize,
        /// Whether this method has a default impl in the trait.
        has_default: bool,
    },
}

/// How the method receives `self`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Receiver {
    None,
    Ref,
    Mut,
    Value,
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// A method parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    /// Parameter name as written in the source.
    pub name: String,
    /// The type and how it's accessed.
    #[serde(flatten)]
    pub param_type: ParamType,
}

/// The type of a parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "param_kind", rename_all = "snake_case")]
pub enum ParamType {
    /// Normal parameter — one Rust param maps to one C param.
    Regular(TypeRef),
    /// Slice parameter — one Rust `&[T]` param expanded into N C params
    /// (typically pointer + length). The `c_params` field describes exactly
    /// what the bridge generated at the C ABI level.
    Slice {
        /// Element type reference (e.g. `{ type: "str", ref_kind: "shared" }` for `&[&str]`,
        /// or `{ type: "u8" }` for `&[u8]`).
        element: TypeRef,
        /// The C parameters this single Rust param expanded into.
        /// Consumers use these directly without inferring expansion rules.
        c_params: Vec<CParam>,
    },
    /// `impl Trait` parameter.
    ImplTrait {
        /// Trait name — key into `type_registry`.
        trait_name: String,
        /// Lifetime arguments on the trait at this usage site
        /// (e.g. `["a"]` for `impl Snapshot<'a>`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        type_args: Vec<String>,
    },
}

/// A C-level parameter produced by slice expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CParam {
    /// C parameter name (e.g. `"tags"`, `"tags_len"`).
    pub name: String,
    /// C type (e.g. `"const FtStr*"`, `"uintptr_t"`).
    pub c_type: String,
}

// ---------------------------------------------------------------------------
// Return types
// ---------------------------------------------------------------------------

/// Return type of a method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "return_kind", rename_all = "snake_case")]
pub enum Return {
    /// Returns nothing.
    Void,
    /// Returns a single value.
    Value(TypeRef),
    /// Returns `Result<T, E>`.
    Result {
        /// The Ok type. `None` when `Result<(), E>`.
        ok: Option<TypeRef>,
        /// Error type name — key into `type_registry`.
        err_type: String,
    },
}

/// Well-known type name for builder methods that return `Self`.
pub const BUILDER_SELF_TYPE: &str = "BuilderSelf";

/// Check whether a type name refers to a `ReplacesSelf` sentinel in the registry.
fn is_replaces_self(type_name: &str, registry: &BTreeMap<String, TypeEntry>) -> bool {
    registry
        .get(type_name)
        .is_some_and(|e| e.kind == TypeKind::ReplacesSelf)
}

impl Return {
    /// True if this return type represents a builder pattern (`-> Self` or
    /// `-> Result<Self, E>`), detected via `TypeKind::ReplacesSelf` in the
    /// type registry.
    pub fn is_builder_self(&self, registry: &BTreeMap<String, TypeEntry>) -> bool {
        match self {
            Return::Value(tr) => is_replaces_self(&tr.type_name, registry),
            Return::Result { ok: Some(tr), .. } => is_replaces_self(&tr.type_name, registry),
            _ => false,
        }
    }

    /// Reconstruct the Rust return type syntax.
    /// Types with `TypeKind::ReplacesSelf` in the registry render as `Self`.
    pub fn to_rust_type(&self, registry: &BTreeMap<String, TypeEntry>) -> String {
        match self {
            Return::Void => "()".to_string(),
            Return::Value(tr) if is_replaces_self(&tr.type_name, registry) => "Self".to_string(),
            Return::Value(tr) => tr.to_rust_type(),
            Return::Result { ok, err_type } => {
                let ok_str = match ok {
                    Some(tr) if is_replaces_self(&tr.type_name, registry) => "Self".to_string(),
                    Some(tr) => tr.to_rust_type(),
                    None => "()".to_string(),
                };
                format!("Result<{ok_str}, {err_type}>")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// An error enum exported via `#[derive(FfiError)]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorType {
    /// Rust enum name — key into `type_registry`.
    pub name: String,
    pub variants: Vec<ErrorVariant>,
}

/// A variant of an error enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorVariant {
    /// Variant name (e.g. "NotFound").
    pub name: String,
    /// C constant name (e.g. "FT_ERROR_TEST_NOT_FOUND").
    pub c_name: String,
    /// Numeric error code from `#[ffier(code = N)]`.
    pub code: u32,
    /// Human-readable message.
    pub message: String,
}

// ---------------------------------------------------------------------------
// Enum constants
// ---------------------------------------------------------------------------

/// A plain enum exported as C `#define` constants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumType {
    /// Rust enum name — key into `type_registry`.
    pub name: String,
    pub variants: Vec<EnumVariant>,
}

/// A variant of an enum constant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumVariant {
    /// Variant name (e.g. "Off").
    pub name: String,
    /// C constant name (e.g. "FT_LOG_LEVEL_OFF").
    pub c_name: String,
    /// Numeric discriminant value.
    pub value: u64,
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// A free function exported via `#[ffier::exportable]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeFunction {
    /// Rust function name (e.g. "init_log").
    pub name: String,
    /// C FFI function name (e.g. "ft_init_log").
    pub ffi_name: String,
    /// Doc comment lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Return,
}

// ---------------------------------------------------------------------------
// Implementable traits
// ---------------------------------------------------------------------------

/// A trait exported via `#[ffier::implementable]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplementableTrait {
    /// Trait name — key into `type_registry`.
    pub name: String,
    /// FFI destroy/dispatch function name (e.g. `"ft_fruit_destroy"`).
    pub destroy_ffi_name: String,
    /// C constant name for the vtable handle type tag
    /// (e.g. `"FT_PUSH_STR_TYPE_TAG"`).
    pub type_tag_constant: String,
    /// Optional pragma tag for well-known builtin traits.
    /// E.g. `"error_trait"` for the `Error` trait, allowing generators to
    /// locate it without hardcoding the trait name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pragma: Option<String>,
    pub methods: Vec<Method>,
    /// Number of methods that belong to this trait (not supertrait methods).
    pub own_method_count: usize,
    /// Highest vtable slot index (including reserved/retired slots).
    pub max_vtable_slot: usize,
}

// ---------------------------------------------------------------------------
// Trait impls
// ---------------------------------------------------------------------------

/// An `impl Trait for Struct` exported via `#[ffier::trait_impl]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraitImpl {
    /// Trait name — key into `type_registry`.
    pub trait_name: String,
    /// Struct name — key into `type_registry`.
    pub struct_name: String,
    /// Lifetime params on the impl block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifetimes: Vec<String>,
    /// Lifetime args on the trait.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trait_lifetime_args: Vec<String>,
    /// Lifetime args on the struct.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub struct_lifetime_args: Vec<String>,
    pub methods: Vec<Method>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_ref_none(v: &RefKind) -> bool {
    *v == RefKind::None
}

impl Library {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("failed to serialize library metadata")
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Look up a type entry by name.
    pub fn type_entry(&self, name: &str) -> Option<&TypeEntry> {
        self.type_registry.get(name)
    }

    /// Get the C type name for a type by its registry key, resolving
    /// through alias/enum chains to the concrete C type.
    /// Panics if the type is not in the registry or if the alias chain
    /// exceeds 16 hops (indicates a cycle).
    pub fn c_type_of(&self, name: &str) -> &str {
        const MAX_DEPTH: usize = 16;
        let mut current = name;
        for _ in 0..MAX_DEPTH {
            let entry = self
                .type_registry
                .get(current)
                .unwrap_or_else(|| panic!("type `{current}` not found in type_registry"));
            match &entry.kind {
                TypeKind::Alias { alias_of, .. } | TypeKind::Enum { alias_of } => {
                    current = alias_of;
                }
                _ => return &entry.c_type,
            }
        }
        panic!("alias chain for `{name}` exceeds {MAX_DEPTH} hops — probable cycle");
    }
}
