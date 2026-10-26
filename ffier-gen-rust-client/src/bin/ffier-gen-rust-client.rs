use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut json_file = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            arg if arg.starts_with('-') => {
                eprintln!("unknown flag: {arg}");
                eprintln!("Usage: ffier-gen-rust-client <json-file>");
                process::exit(1);
            }
            _ => {
                if json_file.is_some() {
                    eprintln!("unexpected argument: {}", args[i]);
                    eprintln!("Usage: ffier-gen-rust-client <json-file>");
                    process::exit(1);
                }
                json_file = Some(&args[i]);
            }
        }
        i += 1;
    }

    let Some(json_file) = json_file else {
        eprintln!("Usage: ffier-gen-rust-client <json-file>");
        process::exit(1);
    };

    match ffier_gen_rust_client::generate_from_file(json_file) {
        Ok(src) => print!("{src}"),
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}
