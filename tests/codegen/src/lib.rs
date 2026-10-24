//! Shared code generation helpers for ffier test crates.
//!
//! Test crates call these from their `build.rs` to produce client bindings
//! and C headers on the fly from the JSON schemas emitted by the test
//! libraries' `ffier::generate_bridge!` calls. Previously the generated
//! sources were committed as snapshots; they are now derived at build time
//! so test changes no longer require committing thousands of generated
//! lines. CI renders generated-output diffs into the job summary instead
//! (see tests/render-codegen-diff.py).

use std::path::Path;

use ffier_schema::Library;

/// Errors returned by schema loading and code generation.
#[derive(Debug)]
pub enum CodegenError {
    /// Schema file missing — the test library has not been built yet.
    MissingSchema(String),
    /// Schema file could not be read or parsed.
    InvalidSchema(String),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::MissingSchema(p) => write!(
                f,
                "schema not found: {p}\n\
                 Build the test library first: just --justfile tests/justfile build"
            ),
            CodegenError::InvalidSchema(e) => write!(f, "invalid schema: {e}"),
        }
    }
}

impl std::error::Error for CodegenError {}

/// Load a library schema from `target/<name>`, declaring
/// `cargo:rerun-if-changed` so cargo rebuilds when the schema changes.
pub fn load_schema(workspace_root: &Path, schema_name: &str) -> Result<Library, CodegenError> {
    let path = workspace_root.join("target").join(schema_name);
    println!("cargo:rerun-if-changed={}", path.display());
    let json = std::fs::read_to_string(&path)
        .map_err(|_| CodegenError::MissingSchema(path.display().to_string()))?;
    Library::from_json(&json).map_err(|e| CodegenError::InvalidSchema(e.to_string()))
}

/// Generate Rust client bindings and write them to `OUT_DIR/generated.rs`.
/// Returns the generated source. The generator formats its own output.
pub fn write_rust_client(
    lib: &Library,
    out_dir: &Path,
    transform: Option<fn(&mut Library)>,
) -> Result<String, CodegenError> {
    let mut lib = lib.clone();
    if let Some(transform) = transform {
        transform(&mut lib);
    }
    let src = ffier_gen_rust_client::generate(&lib);
    let out = out_dir.join("generated.rs");
    std::fs::write(&out, &src)
        .map_err(|e| CodegenError::InvalidSchema(format!("write {}: {e}", out.display())))?;
    // Shared harnesses explicitly initialize the test libraries' API. Their
    // Optional* entries are tested separately, so omit them from this list.
    let variants = src
        .split("pub enum Symbol {\n")
        .nth(1)
        .unwrap()
        .split("\n}")
        .next()
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|line| line.ends_with(',') && !line.contains("Optional"))
        .map(|line| format!("Symbol::{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        out_dir.join("required_symbols.rs"),
        format!("&[\n{variants}\n]"),
    )
    .map_err(|e| CodegenError::InvalidSchema(e.to_string()))?;
    Ok(src)
}

/// Backward-compatibility transform: remove `label` from the `Fruit` trait.
///
/// The compat crate links v1-style client bindings (no `Fruit::label`,
/// slot reserved) against the current v2 library to verify the empty-slot
/// default-dispatch fallback.
pub fn remove_fruit_label(lib: &mut Library) {
    for trait_ in &mut lib.traits {
        if trait_.name == "Fruit" {
            trait_.methods.retain(|m| m.name != "label");
            trait_.own_method_count = trait_.methods.len();
        }
    }
}
