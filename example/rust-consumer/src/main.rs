// Swap between native Rust linking and C ABI dynamic linking:
//   cargo run -p mylib-rust-consumer --no-default-features --features native
//   cargo run -p mylib-rust-consumer --no-default-features --features via-cdylib

#[cfg(feature = "native")]
use mylib as api;

#[cfg(feature = "via-cdylib")]
use mylib_via_cdylib as api;

use api::{Calculator, TextBuffer};
use std::os::unix::io::AsFd;

fn main() {
    // ---- Calculator ----
    let calc = Calculator::new();

    println!("add(3, 4) = {}", calc.add(3, 4));
    println!("is_positive(-1) = {}", calc.is_positive(-1));

    println!("divide(10, 3) = {}", calc.divide(10, 3).unwrap());
    assert!(calc.divide(1, 0).is_err());

    // ---- TextBuffer ----
    let stdout_dup = std::io::stdout().as_fd().try_clone_to_owned().unwrap();
    let mut buf = TextBuffer::new(stdout_dup);

    buf.write("hello ");
    buf.write_parts(&["wo", "rld"]);

    println!("buffer = {}", buf.contents());

    let raw = buf.as_bytes();
    assert_eq!(raw.len(), 11);

    buf.flush().unwrap();
    println!();

    buf.clear();

    println!("done.");
}
