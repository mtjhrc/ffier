ffier_test_lib::ffier_meta_op_widget!("ft", ffier::generate_bridge);
ffier_test_lib::ffier_meta_op_gadget!("ft", ffier::generate_bridge);
ffier_test_lib::ffier_meta_op_config!("ft", ffier::generate_bridge);
ffier_test_lib::ffier_meta_op_gizmo!("ft", ffier::generate_bridge);
ffier_test_lib::ffier_meta_op_gizmo_builder!("ft", ffier::generate_bridge);
ffier_test_lib::ffier_meta_op_view!("ft", ffier::generate_bridge);
ffier_test_lib::ffier_meta_op_pipeline!("ft", ffier::generate_bridge);
ffier_test_lib::ffier_meta_op_test_error!("ft", ffier::generate_bridge);
ffier_test_lib::ffier_meta_op_vtable_processor!("ft", ffier::generate_bridge);

fn main() {
    let header = ffier::HeaderBuilder::new("FFIER_TEST_H")
        .add(ft_test_error__header())
        .add(ft_widget__header())
        .add(ft_gadget__header())
        .add(ft_config__header())
        .add(ft_gizmo__header())
        .add(ft_gizmo_builder__header())
        .add(ft_view__header())
        .add(ft_pipeline__header())
        .add(ft_vtable_processor__header())
        .build();
    print!("{header}");
}
