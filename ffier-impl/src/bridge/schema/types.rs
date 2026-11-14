//! Resolve Rust metadata types into schema type references and C names.

use quote::{ToTokens, quote};

use crate::meta::camel_to_upper_snake;

/// Context for C type resolution during schema conversion.
pub(super) struct CTypeResolver {
    pub(super) type_pfx: String,      // e.g. "Ft"
    pub(super) prim_type_pfx: String, // e.g. "Ft" (or "Krun" when primitives_prefix differs)
    pub(super) upper_pfx: String,     // e.g. "FT_"
    fn_pfx: String,                   // e.g. "ft_"
}

impl CTypeResolver {
    pub(super) fn new(prefix: &str, primitives_prefix: Option<&str>) -> Self {
        let type_pfx = crate::meta::snake_to_pascal(prefix);
        let prim_type_pfx = crate::meta::snake_to_pascal(primitives_prefix.unwrap_or(prefix));
        let upper_pfx = format!("{}_", prefix.to_ascii_uppercase());
        let fn_pfx = format!("{prefix}_");
        CTypeResolver {
            type_pfx,
            prim_type_pfx,
            upper_pfx,
            fn_pfx,
        }
    }

    /// Resolve the C type for a handle (always opaque pointer typedef).
    pub(super) fn handle_c_name(&self, name: &str) -> String {
        format!("{}{}", self.type_pfx, name)
    }

    /// FFI function name: prefix + ffi_name suffix.
    pub(super) fn ffi_fn_name(&self, ffi_name: &str) -> String {
        format!("{}{}", self.fn_pfx, ffi_name)
    }

    /// Error constant name: FT_ERROR_TEST_NOT_FOUND
    /// Strips "Error" suffix from the error type name: TestError → TEST.
    pub(super) fn error_const_name(&self, error_name: &str, variant_name: &str) -> String {
        let stripped = error_name.strip_suffix("Error").unwrap_or(error_name);
        let error_upper = camel_to_upper_snake(stripped);
        let variant_upper = camel_to_upper_snake(variant_name);
        format!("{}ERROR_{}_{}", self.upper_pfx, error_upper, variant_upper)
    }

    /// Build a `TypeRef` from a parsed `syn::Type`, extracting references,
    /// lifetimes, `Option<_>`, `Box<str>`, qualified paths, and generic
    /// lifetime arguments structurally — without stringifying and re-parsing.
    ///
    /// Types that have no schema representation are rejected here rather than
    /// stringified: a stringified name is absent from the type registry, so it
    /// only failed later as an unrelated "type not found" panic.
    pub(super) fn type_ref_from_syn(&self, ty: &syn::Type) -> syn::Result<ffier_schema::TypeRef> {
        // A `macro_rules!` `$t:ty` capture arrives as an invisible
        // `Delimiter::None` group. It does not change the type, so resolve what
        // it wraps.
        let ty = unwrap_invisible_group(ty);
        match ty {
            // Option<T> → recurse into T and set optional = true.
            syn::Type::Path(tp) if crate::types::is_option(tp) => {
                let inner = crate::types::option_inner(tp);
                let mut tr = self.type_ref_from_syn(inner)?;
                tr.optional = true;
                Ok(tr)
            }

            // Box<str> → owned str.
            syn::Type::Path(tp) if crate::types::is_box_str(tp) => Ok(ffier_schema::TypeRef {
                type_name: "str".to_string(),
                ref_kind: ffier_schema::RefKind::None,
                ref_lifetime: None,
                type_args: vec![],
                optional: false,
                owned: true,
            }),

            // &T or &mut T — extract reference kind, lifetime, then recurse.
            syn::Type::Reference(ref_ty) => {
                let ref_kind = if ref_ty.mutability.is_some() {
                    ffier_schema::RefKind::Mut
                } else {
                    ffier_schema::RefKind::Shared
                };
                let ref_lifetime = ref_ty.lifetime.as_ref().map(|lt| lt.ident.to_string());
                // A capture written into a reference (`&$t`) groups the
                // pointee, so unwrap it before classifying the target.
                match unwrap_invisible_group(ref_ty.elem.as_ref()) {
                    syn::Type::Path(tp) => {
                        Ok(self.type_ref_from_path(&tp.path, ref_kind, ref_lifetime))
                    }
                    // &[u8] slice — type name is "[u8]".
                    syn::Type::Slice(sl) => {
                        let elem_str = sl.elem.to_token_stream().to_string().replace(' ', "");
                        Ok(ffier_schema::TypeRef {
                            type_name: format!("[{elem_str}]"),
                            ref_kind,
                            ref_lifetime,
                            type_args: vec![],
                            optional: false,
                            owned: false,
                        })
                    }
                    // &str is a path and was handled above. Anything else
                    // (`&dyn Trait`, `&(A, B)`, ...) has no schema shape.
                    _ => Err(unsupported_ffi_type(ty)),
                }
            }

            // Bare path: i32, str, Widget, View<'a>,
            // ffier_test_foreign_lib::ForeignConfig, etc.
            syn::Type::Path(tp) => {
                Ok(self.type_ref_from_path(&tp.path, ffier_schema::RefKind::None, None))
            }

            // *mut c_void / *const c_void → opaque pointer primitives.
            syn::Type::Ptr(ptr) if is_c_void(&ptr.elem) => {
                let type_name = if ptr.mutability.is_some() {
                    ffier_schema::C_VOID_PTR
                } else {
                    ffier_schema::C_VOID_CONST_PTR
                };
                Ok(ffier_schema::TypeRef {
                    type_name: type_name.to_string(),
                    ref_kind: ffier_schema::RefKind::None,
                    ref_lifetime: None,
                    type_args: vec![],
                    optional: false,
                    owned: false,
                })
            }

            // The unit type is the only tuple with a schema spelling; it
            // appears as a typed (non-opaque) error payload field.
            syn::Type::Tuple(tuple) if tuple.elems.is_empty() => Ok(ffier_schema::TypeRef {
                type_name: "()".to_string(),
                ref_kind: ffier_schema::RefKind::None,
                ref_lifetime: None,
                type_args: vec![],
                optional: false,
                owned: false,
            }),

            // Arrays, non-unit tuples, typed raw pointers, trait objects, bare
            // fn pointers, and everything else have no schema representation. A
            // typed raw pointer is not stringified to its pointee: that would
            // claim a `*mut u8` slot holds a plain `u8`.
            _ => Err(unsupported_ffi_type(ty)),
        }
    }

    /// Build a `TypeRef` from a `syn::Path`, extracting the last segment
    /// as the type name and any lifetime arguments from angle brackets.
    ///
    /// For qualified paths like `ffier_test_foreign_lib::ForeignConfig`,
    /// this uses the **last segment** (`ForeignConfig`) as the type name —
    /// no spaces-around-`::` nonsense.
    fn type_ref_from_path(
        &self,
        path: &syn::Path,
        ref_kind: ffier_schema::RefKind,
        ref_lifetime: Option<String>,
    ) -> ffier_schema::TypeRef {
        let last = path.segments.last().expect("empty path");
        let type_name = last.ident.to_string();

        // Extract lifetime arguments from angle brackets: View<'a> → ["a"].
        let type_args = match &last.arguments {
            syn::PathArguments::AngleBracketed(ab) => ab
                .args
                .iter()
                .filter_map(|arg| {
                    if let syn::GenericArgument::Lifetime(lt) = arg {
                        Some(lt.ident.to_string())
                    } else {
                        None
                    }
                })
                .collect(),
            _ => vec![],
        };

        ffier_schema::TypeRef {
            type_name,
            ref_kind,
            ref_lifetime,
            type_args,
            optional: false,
            owned: false,
        }
    }

    /// Build a `TypeRef` from a `proc_macro2::TokenStream` by parsing it
    /// as a `syn::Type` first. This is the primary entry point for converting
    /// metadata type tokens to schema type references.
    pub(super) fn type_ref_from_tokens(
        &self,
        tokens: &proc_macro2::TokenStream,
    ) -> syn::Result<ffier_schema::TypeRef> {
        let ty: syn::Type = syn::parse2(tokens.clone())
            .unwrap_or_else(|e| panic!("failed to parse type tokens `{tokens}`: {e}"));
        self.type_ref_from_syn(&ty)
    }
}

/// Diagnostic for a Rust type that has no schema representation.
///
/// Such a type is absent from the type registry, so accepting it only deferred
/// the failure to an unrelated `type ... not found in type_registry` panic.
fn unsupported_ffi_type(ty: &syn::Type) -> syn::Error {
    // `ToTokens` separates `*`/`&` from the pointee, but Rust spells them
    // without a space. No valid type needs `* ` or `& `.
    let rendered = quote!(#ty)
        .to_string()
        .replace("* ", "*")
        .replace("& ", "&");
    syn::Error::new_spanned(
        ty,
        format!(
            "ffier: unsupported FFI type `{rendered}`; expected a scalar, `&str`, `Box<str>`, \
             `&[u8]`, `&mut [u8]`, `&[T]`, `&[&T]`, `Option<T>`, `*mut c_void`, \
             `*const c_void`, or a registered handle, value struct, enum, or bitflags type"
        ),
    )
}

/// Strip the invisible `Delimiter::None` groups `macro_rules!` puts around a
/// `$t:ty` capture. They carry no syntax of their own; nesting can occur when a
/// capture is itself substituted into another capture's tokens.
fn unwrap_invisible_group(mut ty: &syn::Type) -> &syn::Type {
    while let syn::Type::Group(group) = ty {
        ty = &group.elem;
    }
    ty
}

/// Check if a `syn::Type` is `c_void` or `core::ffi::c_void` (any qualification).
fn is_c_void(ty: &syn::Type) -> bool {
    if let syn::Type::Path(tp) = ty {
        let last = tp.path.segments.last();
        last.is_some_and(|seg| seg.ident == "c_void" && seg.arguments.is_none())
    } else {
        false
    }
}
