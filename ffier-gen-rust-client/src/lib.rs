//! Rust client bindings generator from ffier JSON schema.
//!
//! Reads a `ffier-{prefix}.json` and produces a complete Rust source file with:
//! - Error enums with `from_ffi`, `Display`, `Error` impls
//! - Handle wrapper structs with extern "C" declarations
//! - Safe method wrappers with lifetime-correct signatures
//! - Trait definitions with vtable structs for exported traits
//! - Trait impl blocks for concrete types

mod abi;
mod calls;
mod cfg;
mod error;
mod ffi;
mod formatting;
mod functions;
mod generate;
mod methods;
mod signature;
mod thread_safety;
mod traits;
mod types;

use ffier_schema::Library;

pub use generate::generate;

/// Generate from a JSON file path.
pub fn generate_from_file(json_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(json_path)?;
    let lib = Library::from_json(&json)?;
    Ok(generate(&lib))
}
