// FIXME(generator-dead-code): generator emits ForeignSlice and error_payload
// unconditionally even when unused. Remove once the generator is fixed.
#![allow(dead_code)]

include!("generated.rs");

#[cfg(test)]
mod tests {
    use super::ForeignError;

    #[test]
    fn fieldless_display_uses_variant_name_when_static_message_is_omitted() {
        assert_eq!(ForeignError::Invalid().to_string(), "Invalid");
    }
}
