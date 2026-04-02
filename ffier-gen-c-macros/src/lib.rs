use proc_macro::TokenStream;

/// Generates C FFI bridge functions from batched ffier metadata.
///
/// Receives `{ @tag, ... } { @tag, ... } ...` — multiple metadata items
/// produced by the `__ffier_{prefix}_library!` recursive fold. Sorts items by
/// category and generates all bridge code + a unified header function.
#[proc_macro]
pub fn generate(input: TokenStream) -> TokenStream {
    ffier_gen_c::generate_batch_impl(input.into()).into()
}
