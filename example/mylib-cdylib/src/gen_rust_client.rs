mylib::__ffier_meta_calculator!("mylib", ffier_gen_rust::generate_client_source);
mylib::__ffier_meta_calc_error!("mylib", ffier_gen_rust::generate_client_source);
mylib::__ffier_meta_text_buffer!("mylib", ffier_gen_rust::generate_client_source);
mylib::__ffier_meta_buffer_error!("mylib", ffier_gen_rust::generate_client_source);

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
