ffier_test_lib::__ffier_meta_widget!(ffier::generate_bridge);
ffier_test_lib::__ffier_meta_gadget!(ffier::generate_bridge);
ffier_test_lib::__ffier_meta_config!(ffier::generate_bridge);
ffier_test_lib::__ffier_meta_gizmo!(ffier::generate_bridge);
ffier_test_lib::__ffier_meta_gizmo_builder!(ffier::generate_bridge);
ffier_test_lib::__ffier_meta_view!(ffier::generate_bridge);
ffier_test_lib::__ffier_meta_pipeline!(ffier::generate_bridge);
ffier_test_lib::__ffier_meta_test_error!("ft_", "Ft", "FT_", ffier::generate_bridge);
ffier_test_lib::__ffier_meta_vtable_processor!(ffier::generate_bridge);

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
