example_lib::ffier_meta_op_calculator!("mylib", ffier_gen_c::generate_bridge);
example_lib::ffier_meta_op_calc_result!("mylib", ffier_gen_c::generate_bridge);
example_lib::ffier_meta_op_calc_error!("mylib", ffier_gen_c::generate_bridge);
example_lib::ffier_meta_op_text_buffer!("mylib", ffier_gen_c::generate_bridge);
example_lib::ffier_meta_op_buffer_error!("mylib", ffier_gen_c::generate_bridge);

fn main() {
    let header = ffier::HeaderBuilder::new("MYLIB_H")
        .add(mylib_calc_error__header())
        .add(mylib_buffer_error__header())
        .add(mylib_calc_result__header())
        .add(mylib_calculator__header())
        .add(mylib_text_buffer__header())
        .build();
    print!("{header}");
}
