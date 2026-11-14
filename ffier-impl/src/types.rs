//! Shared type classification and rewriting for export and bridge generation.

use quote::ToTokens;
use syn::{GenericArgument, PathArguments, Type, visit_mut::VisitMut};

/// Detect `&[&str]` — a slice of string references.
pub(crate) fn is_str_slice(ty: &Type) -> bool {
    let Type::Reference(ref_ty) = ty else {
        return false;
    };
    let Type::Slice(sl) = &*ref_ty.elem else {
        return false;
    };
    let Type::Reference(inner_ref) = &*sl.elem else {
        return false;
    };
    matches!(&*inner_ref.elem, Type::Path(tp) if tp.path.is_ident("str"))
}

/// Check for types that should not be used across FFI and return a
/// compile error message if found. Returns `None` if the type is fine.
pub(crate) fn check_forbidden_ffi_type(ty: &Type) -> Option<&'static str> {
    // Peel through references
    let inner = match ty {
        Type::Reference(r) => &*r.elem,
        _ => ty,
    };
    let Type::Path(tp) = inner else { return None };
    let last = tp.path.segments.last()?;
    let name = last.ident.to_string();
    match name.as_str() {
        "String" => {
            return Some(
                "String cannot be used in FFI — use `Box<str>` (owned) or `&str` (borrowed) instead",
            );
        }
        "Vec" => {
            return Some(
                "Vec<T> cannot be used in FFI — use `&[T]` (borrowed slice) or `Box<[T]>` instead",
            );
        }
        _ => {}
    }
    // Recurse into generic args (Result<String, E>, Option<Vec<T>>, etc.)
    if let syn::PathArguments::AngleBracketed(args) = &last.arguments {
        for arg in &args.args {
            if let syn::GenericArgument::Type(inner_ty) = arg
                && let Some(msg) = check_forbidden_ffi_type(inner_ty)
            {
                return Some(msg);
            }
        }
    }
    None
}

/// Detect `&[&T]` or `&[T]` where T is a non-primitive named type
/// (i.e. an exported handle type). Returns the element type T if matched.
/// Excludes `&[&str]` (handled as StrSlice).
pub(crate) fn handle_slice_elem(ty: &Type) -> Option<&Type> {
    let Type::Reference(ref_ty) = ty else {
        return None;
    };
    let Type::Slice(sl) = &*ref_ty.elem else {
        return None;
    };
    // Peel off inner & if present: &[&T] → T, &[T] → T
    let elem = match &*sl.elem {
        Type::Reference(inner_ref) => &*inner_ref.elem,
        other => other,
    };
    // Not &[&str] / &[str] (already handled by is_str_slice)
    if matches!(elem, Type::Path(tp) if tp.path.is_ident("str")) {
        return None;
    }
    // Must be a named type that isn't a primitive
    let Type::Path(tp) = elem else {
        return None;
    };
    let name = tp
        .path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default();
    if PRIMITIVES.contains(&name.as_str()) {
        return None;
    }
    Some(elem)
}

/// Compat wrapper — returns true for `&[&T]` or `&[T]` handle slices.
pub(crate) fn is_handle_slice(ty: &Type) -> bool {
    handle_slice_elem(ty).is_some()
}

/// Detect `&[T]` where T is a primitive type (u32, i64, bool, etc.).
/// Returns the element type T if matched.
/// Excludes `&[u8]` (already handled as `FfierBytes`/bytes).
pub(crate) fn primitive_slice_elem(ty: &Type) -> Option<&Type> {
    let Type::Reference(ref_ty) = ty else {
        return None;
    };
    let Type::Slice(sl) = &*ref_ty.elem else {
        return None;
    };
    // Must not have an inner reference — &[T] not &[&T]
    if matches!(&*sl.elem, Type::Reference(_)) {
        return None;
    }
    let Type::Path(tp) = &*sl.elem else {
        return None;
    };
    let name = tp
        .path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default();
    // Must be a primitive, but not u8 (which is already handled as bytes)
    if name == "u8" || !PRIMITIVES.contains(&name.as_str()) {
        return None;
    }
    Some(&*sl.elem)
}

pub(crate) fn extract_result_types(ty: &Type) -> Option<(Type, Type)> {
    let Type::Path(tp) = ty else { return None };
    let last = tp.path.segments.last()?;
    if !is_builtin_type_path(tp, "Result") {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    if args.args.len() != 2 {
        return None;
    }
    let mut iter = args.args.iter();
    let GenericArgument::Type(ok_ty) = iter.next()? else {
        return None;
    };
    let GenericArgument::Type(err_ty) = iter.next()? else {
        return None;
    };
    Some((ok_ty.clone(), err_ty.clone()))
}

pub(crate) fn type_ident_name(ty: &Type) -> String {
    let Type::Path(tp) = ty else {
        return "Error".to_string();
    };
    tp.path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_else(|| "Error".to_string())
}

pub(crate) fn is_unit_type(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(t) if t.elems.is_empty())
}

/// Check if `ty` is the same type as `target` or a reference to it.
/// Matches: `Self`, `&Self`, `&mut Self` (after Self→concrete replacement).
pub(crate) fn is_self_return(ty: &Type, target: &Type) -> bool {
    let tgt = target.to_token_stream().to_string().replace(' ', "");
    let inner = match ty {
        Type::Reference(ref_ty) => ref_ty.elem.to_token_stream().to_string().replace(' ', ""),
        _ => ty.to_token_stream().to_string().replace(' ', ""),
    };
    inner == tgt
}

/// Replace all occurrences of `Self` in a type with a concrete type.
pub(crate) fn replace_self_type(ty: &Type, replacement: &Type) -> Type {
    struct Replacer<'a>(&'a Type);
    impl VisitMut for Replacer<'_> {
        fn visit_type_mut(&mut self, ty: &mut Type) {
            if let Type::Path(tp) = ty
                && tp.path.is_ident("Self")
            {
                *ty = self.0.clone();
                return;
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }
    let mut ty = ty.clone();
    Replacer(replacement).visit_type_mut(&mut ty);
    ty
}

/// Recognize Option, Box, and Result by their bare or standard-library paths.
pub(crate) fn is_builtin_type_path(tp: &syn::TypePath, name: &str) -> bool {
    if tp.qself.is_some() {
        return false;
    }
    let segments = &tp.path.segments;
    if segments.len() == 1 {
        return tp.path.leading_colon.is_none() && segments[0].ident == name;
    }
    let (module, roots) = match name {
        "Option" => ("option", ["std", "core"]),
        "Box" => ("boxed", ["std", "alloc"]),
        "Result" => ("result", ["std", "core"]),
        _ => return false,
    };
    segments.len() == 3
        && roots.iter().any(|root| segments[0].ident == root)
        && segments[1].ident == module
        && segments[2].ident == name
        && matches!(segments[0].arguments, syn::PathArguments::None)
        && matches!(segments[1].arguments, syn::PathArguments::None)
}

/// Extract the single type argument from `Path<T>`.
fn single_angle_arg(path: &syn::Path) -> Option<&Type> {
    let last = path.segments.last()?;
    if let PathArguments::AngleBracketed(args) = &last.arguments
        && args.args.len() == 1
        && let GenericArgument::Type(ty) = &args.args[0]
    {
        return Some(ty);
    }
    None
}

/// Check if a TypePath is `Option<_>`.
pub(crate) fn is_option(tp: &syn::TypePath) -> bool {
    is_builtin_type_path(tp, "Option") && single_angle_arg(&tp.path).is_some()
}

/// Extract the inner type from `Option<T>`.
pub(crate) fn option_inner(tp: &syn::TypePath) -> &Type {
    single_angle_arg(&tp.path).expect("expected Option<T>")
}

/// Check if a TypePath is `Box<str>`.
pub(crate) fn is_box_str(tp: &syn::TypePath) -> bool {
    is_builtin_type_path(tp, "Box")
        && single_angle_arg(&tp.path)
            .is_some_and(|inner| matches!(inner, Type::Path(p) if p.path.is_ident("str")))
}

const PRIMITIVES: &[&str] = &[
    "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "isize", "usize", "bool", "f32", "f64",
];

pub(crate) fn is_primitive(ty: &Type) -> bool {
    let Type::Path(tp) = ty else { return false };
    tp.path.segments.len() == 1
        && PRIMITIVES.contains(&tp.path.segments[0].ident.to_string().as_str())
}
