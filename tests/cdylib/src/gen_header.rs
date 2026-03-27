use ffier_test_lib::{
    Config, Gadget, Gizmo, GizmoBuilder, Pipeline, View, Widget,
};

ffier_test_lib::widget_ffier!(Widget);
ffier_test_lib::gadget_ffier!(Gadget);
ffier_test_lib::config_ffier!(Config);
ffier_test_lib::gizmo_ffier!(Gizmo);
ffier_test_lib::gizmo_builder_ffier!(GizmoBuilder);
ffier_test_lib::view_ffier!(View<'static>);
ffier_test_lib::pipeline_ffier!(Pipeline);
ffier_test_lib::test_error_error_ffier!("ft");
ffier_test_lib::vtable_processor_ffier!();

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
