use proc_macro::TokenStream;

/// Generates C FFI bridge functions from ffier metadata.
#[proc_macro]
pub fn generate_bridge(input: TokenStream) -> TokenStream {
    ffier_gen_c::generate_bridge_impl(input.into()).into()
}
