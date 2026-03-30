ffier_test_lib::ffier_meta_op_widget!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_gadget!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_config!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_gizmo!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_gizmo_builder!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_view!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_pipeline!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_test_error!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_vtable_processor!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_apple!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_orange!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_vtable_fruit!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::ffier_meta_op_mixer!("ft", ffier_gen_c_macros::generate_bridge);

fn main() {
    let header = ffier_gen_c::HeaderBuilder::new("FFIER_TEST_H")
        .add(ft_test_error__header())
        .add(ft_widget__header())
        .add(ft_gadget__header())
        .add(ft_config__header())
        .add(ft_gizmo__header())
        .add(ft_gizmo_builder__header())
        .add(ft_view__header())
        .add(ft_pipeline__header())
        .add(ft_vtable_processor__header())
        .add(ft_apple__header())
        .add(ft_orange__header())
        .add(ft_vtable_fruit__header())
        .add(ft_mixer__header())
        .build();
    print!("{header}");
}
