use proc_macro::TokenStream;

/// Generates C FFI bridge functions from ffier metadata (single item).
#[proc_macro]
pub fn generate_bridge(input: TokenStream) -> TokenStream {
    ffier_gen_c::generate_bridge_impl(input.into()).into()
}

/// Generates C FFI bridge functions from batched ffier metadata.
///
/// Receives `{ @tag, ... } { @tag, ... } ...` — multiple metadata items
/// produced by the `__ffier_meta_lib!` recursive fold. Sorts items by
/// category and generates all bridge code + a unified header function.
#[proc_macro]
pub fn generate(input: TokenStream) -> TokenStream {
    ffier_gen_c::generate_batch_impl(input.into()).into()
}
