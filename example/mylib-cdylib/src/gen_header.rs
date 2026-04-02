mylib::__ffier_library!(ffier_gen_c_macros::generate_bridge);

fn main() {
    let header = ffier_gen_c::HeaderBuilder::new("MYLIB_H")
        .add(mylib_calc_error__header())
        .add(mylib_buffer_error__header())
        .add(mylib_calculator__header())
        .add(mylib_text_buffer__header())
        .build();
    print!("{header}");
}
