// FIXME(generator-dead-code): generator emits ForeignSlice and error_payload
// unconditionally even when unused. Remove once the generator is fixed.
#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/generated.rs"));
include!("../../load_dll.rs");

#[cfg(test)]
mod tests {
    use super::ForeignError;

    #[test]
    fn fieldless_display_uses_variant_name_when_static_message_is_omitted() {
        assert_eq!(ForeignError::Invalid().to_string(), "Invalid");
    }
}

/// Explicit initialization for the shared client test harnesses.
pub fn init_for_tests() {
    #[cfg(all(windows, not(feature = "strong")))]
    {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| unsafe {
            set_library_handle(
                Some(load_test_dll("ffier_test_foreign_lib")),
                LoadPolicy::Explicit,
            )
            .unwrap();
        });
    }
    require(include!(concat!(env!("OUT_DIR"), "/required_symbols.rs"))).unwrap();
}
