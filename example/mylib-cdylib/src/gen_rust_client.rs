mylib::ffier_meta_op_calculator!("mylib", ffier_gen_rust::generate_client_source);
mylib::ffier_meta_op_calc_result!("mylib", ffier_gen_rust::generate_client_source);
mylib::ffier_meta_op_calc_error!("mylib", ffier_gen_rust::generate_client_source);
mylib::ffier_meta_op_text_buffer!("mylib", ffier_gen_rust::generate_client_source);
mylib::ffier_meta_op_buffer_error!("mylib", ffier_gen_rust::generate_client_source);

fn main() {
    println!("// Auto-generated. Regenerate with: just gen-rust-client");
    println!();
    println!("#[allow(unused_imports)]");
    println!("use std::os::unix::io::{{AsRawFd, BorrowedFd, FromRawFd, OwnedFd}};");
    println!();
    print!("{FFIER_SRC_CALC_ERROR}");
    print!("{FFIER_SRC_BUFFER_ERROR}");
    print!("{FFIER_SRC_CALCULATOR}");
    print!("{FFIER_SRC_CALC_RESULT}");
    print!("{FFIER_SRC_TEXT_BUFFER}");
}
