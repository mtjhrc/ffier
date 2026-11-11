use syn::{FnArg, ReturnType, Signature, Type, parse_quote};

use super::{extract_result_types, is_handle_slice, is_option, option_inner, replace_self_type};

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

pub(super) fn check_borrowed_self(
    signature: &Signature,
    self_type: Option<&Type>,
    is_builder: bool,
) -> syn::Result<()> {
    if !cfg!(feature = "unstable_unsafe_borrowed_self")
        && let ReturnType::Type(_, ty) = &signature.output
        && borrowed_self_requires_feature(ty, self_type, is_builder)
    {
        return Err(syn::Error::new_spanned(
            &signature.output,
            "ffier borrowed Self returns require the `unstable_unsafe_borrowed_self` feature; returned references are not reliably preserved",
        ));
    }
    Ok(())
}

fn borrowed_self_requires_feature(
    return_type: &Type,
    self_type: Option<&Type>,
    is_builder: bool,
) -> bool {
    let mut returned = return_type.clone();
    let mut optional = false;
    let mut result_depth = 0;
    loop {
        match &returned {
            Type::Path(path) if is_option(path) => {
                optional = true;
                returned = option_inner(path).clone();
            }
            Type::Paren(wrapped) => returned = (*wrapped.elem).clone(),
            Type::Group(wrapped) => returned = (*wrapped.elem).clone(),
            _ => match extract_result_types(&returned) {
                Some((ok, _)) => {
                    result_depth += 1;
                    returned = ok;
                }
                None => break,
            },
        }
    }
    let target = self_type.cloned().unwrap_or_else(|| parse_quote!(Self));
    let returned = replace_self_type(&returned, &target);
    let Type::Reference(reference) = returned else {
        return false;
    };
    // Direct static identity and one Result use the ordinary borrowed-handle path.
    is_builder
        || (optional || result_depth > 1 || self_type.is_none())
            && reference.elem.as_ref() == &target
}
