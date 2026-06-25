use std::path::Path;
use std::process;

use clap::Parser;

/// Generate a C header from an ffier JSON schema.
#[derive(Parser)]
struct Cli {
    /// Path to the ffier JSON schema file.
    json_file: String,

    /// Header guard name. Derived from the filename if omitted
    /// (e.g. `ffier-ft.json` becomes `FFIER_FT_H`).
    header_guard: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    let guard = cli
        .header_guard
        .unwrap_or_else(|| default_guard(&cli.json_file));

    match ffier_gen_c_header::generate_from_file(&cli.json_file, &guard) {
        Ok(header) => print!("{header}"),
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

fn default_guard(path: &str) -> String {
    // Extract prefix from filename: ffier-ft.json → FFIER_FT_H
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("FFIER");
    format!("{}_H", stem.to_ascii_uppercase().replace('-', "_"))
}
