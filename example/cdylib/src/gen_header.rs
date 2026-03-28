example_lib::__ffier_meta_my_calculator!(ffier::generate_bridge);
example_lib::__ffier_meta_calc_result!(ffier::generate_bridge);
example_lib::__ffier_meta_calc_error!("ex_", "Ex", "EX_", ffier::generate_bridge);

fn main() {
    let header = ffier::HeaderBuilder::new("EX_H")
        .add(ex_calc_error__header())
        .add(ex_calc_result__header())
        .add(ex_my_calculator__header())
        .build();
    print!("{header}");
}
