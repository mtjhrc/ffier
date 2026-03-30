example_lib::ffier_meta_op_my_calculator!("ex", ffier_gen_c::generate_bridge);
example_lib::ffier_meta_op_calc_result!("ex", ffier_gen_c::generate_bridge);
example_lib::ffier_meta_op_calc_error!("ex", ffier_gen_c::generate_bridge);

fn main() {
    let header = ffier::HeaderBuilder::new("EX_H")
        .add(ex_calc_error__header())
        .add(ex_calc_result__header())
        .add(ex_my_calculator__header())
        .build();
    print!("{header}");
}
