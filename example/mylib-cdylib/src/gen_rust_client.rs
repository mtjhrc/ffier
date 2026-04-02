mylib::__ffier_mylib_library!(ffier_gen_rust::generate_client_source);

fn main() {
    println!("// Auto-generated. Regenerate with: just gen-rust-client");
    println!();
    println!("#[allow(unused_imports)]");
    println!("use std::os::unix::io::{{AsRawFd, BorrowedFd, FromRawFd, OwnedFd}};");
    println!();
    print!("{FFIER_SRC_CALC_ERROR}");
    print!("{FFIER_SRC_BUFFER_ERROR}");
    print!("{FFIER_SRC_CALCULATOR}");
    print!("{FFIER_SRC_TEXT_BUFFER}");
}
