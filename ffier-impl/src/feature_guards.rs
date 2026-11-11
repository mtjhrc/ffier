use syn::{FnArg, ReturnType, Signature};

use super::is_handle_slice;

pub(super) fn check_handle_slices(signature: &Signature) -> syn::Result<()> {
    if !cfg!(feature = "unstable_unsafe_handle_slices")
        && signature
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Typed(param) => Some(&*param.ty),
                FnArg::Receiver(_) => None,
            })
            .chain(match &signature.output {
                ReturnType::Type(_, ty) => Some(&**ty),
                ReturnType::Default => None,
            })
            .any(is_handle_slice)
    {
        return Err(syn::Error::new_spanned(
            signature,
            "ffier handle slices require the `unstable_unsafe_handle_slices` feature; borrowed handle-slice returns do not preserve source lifetimes",
        ));
    }
    Ok(())
}
