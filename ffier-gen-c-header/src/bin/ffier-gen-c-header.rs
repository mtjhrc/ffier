use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        eprintln!("Usage: ffier-gen-c-header <json-file> [header-guard]");
        process::exit(1);
    }

    let json_path = &args[1];
    let guard = args
        .get(2)
        .map_or_else(|| default_guard(json_path), |g| g.clone());

    match ffier_gen_c_header::generate_from_file(json_path, &guard) {
        Ok(header) => print!("{header}"),
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

fn default_guard(path: &str) -> String {
    // Extract prefix from filename: ffier-ft.json → FFIER_FT_H
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("FFIER");
    format!("{}_H", stem.to_ascii_uppercase().replace('-', "_"))
}
