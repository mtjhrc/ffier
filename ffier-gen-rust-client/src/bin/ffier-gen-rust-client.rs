use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: ffier-gen-rust-client <json-file>");
        process::exit(1);
    }

    match ffier_gen_rust_client::generate_from_file(&args[1]) {
        Ok(src) => print!("{src}"),
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}
