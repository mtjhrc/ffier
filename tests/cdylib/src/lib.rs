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
