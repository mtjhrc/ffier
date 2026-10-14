//! Reexport Widget through provider -> facade -> bridge, then register it here.
//! This crate depends only on the facade, so both cross-crate reexports must
//! forward the metadata and helpers. All three crates share a_feature.

#![deny(warnings)]

#[ffier::reexport]
pub use conditional_facade::Widget;

// The entry gates FfiHandle/FfiType impls here. The provider's export predicate
// also reaches bridge generation, which evaluates it in this crate.
ffier::library_definition!(
    "conditional",
    library_tag = 3,
    #[cfg(feature = "a_feature")]
    Widget = 1,
);

ffier::generate_bridge!(
    local = __ffier_conditional_metadata,
    schema_output = "../../../target/conditional-bridge/schema.json"
);

#[cfg(all(test, feature = "a_feature"))]
mod tests {
    use core::ffi::c_void;

    // Conditional bridge functions live inside a generated cfg-gated block.
    // Call their exported C symbols, just as an FFI client would.
    unsafe extern "C" {
        fn conditional_widget_new(value: i32) -> *mut c_void;
        fn conditional_widget_get(widget: *mut c_void) -> i32;
        fn conditional_widget_destroy(widget: *mut c_void);
    }

    #[test]
    fn enabled_feature_exports_working_widget_through_chained_reexports() {
        unsafe {
            let widget = conditional_widget_new(42);
            assert!(!widget.is_null());
            assert_eq!(conditional_widget_get(widget), 42);
            conditional_widget_destroy(widget);
        }
    }
}
