ffier_test_lib::__ffier_meta_widget!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_gadget!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_config!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_gizmo!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_gizmo_builder!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_view!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_pipeline!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_test_error!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_vtable_processor!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_apple!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_orange!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_vtable_fruit!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_fruit_for_apple!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_fruit_for_orange!("ft", ffier_gen_c_macros::generate_bridge);
ffier_test_lib::__ffier_meta_mixer!("ft", ffier_gen_c_macros::generate_bridge);

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
        .add(ft_fruit_for_apple__header())
        .add(ft_fruit_for_orange__header())
        .add(ft_mixer__header())
        .build();
    print!("{header}");
}
