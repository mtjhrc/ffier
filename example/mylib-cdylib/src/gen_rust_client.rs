mylib::__ffier_mylib_library!(ffier_gen_rust::generate);

fn main() {
    println!("// Auto-generated. Regenerate with: just gen-rust-client");
    println!();
    println!("#[allow(unused_imports)]");
    println!("use std::os::unix::io::{{AsRawFd, BorrowedFd, FromRawFd, OwnedFd}};");
    println!();
    print!("{FFIER_ALL_CLIENT_SRC}");
}
