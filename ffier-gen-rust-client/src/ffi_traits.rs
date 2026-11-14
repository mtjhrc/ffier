//! Spelling the shared `ffier` conversion traits in generated code.
//!
//! The traits live in `ffier-rt` and take a token type. A generated client
//! defines its own `__FfierLocalToken` and implements the traits for its
//! types with it. A foreign handle type comes from another generated client
//! crate and implements them with that crate's token.

use ffier_schema::{Library, TypeKind, TypeRef};

/// The token type each generated client defines at its root.
pub(crate) const TOKEN: &str = "__FfierLocalToken";

/// The client's alias for `<T as FfiType<TOKEN>>::CRepr`. Reserved
/// (`_`-prefixed) so it cannot collide with an exported type.
pub(crate) const C_REPR_ALIAS: &str = "__FfierCRepr";

/// The token whose impls cover `tr`: the foreign client's for a foreign
/// handle, this client's for everything else.
pub(crate) fn token(tr: &TypeRef, lib: &Library) -> String {
    match lib.type_entry(&tr.type_name).map(|e| &e.kind) {
        Some(TypeKind::ForeignHandle { foreign_crate, .. }) => format!("{foreign_crate}::{TOKEN}"),
        _ => TOKEN.to_string(),
    }
}

/// The C representation of `ty`. Shortened to the client's
/// [`C_REPR_ALIAS`] for its own token.
pub(crate) fn c_repr(ty: &str, tok: &str) -> String {
    if tok == TOKEN {
        format!("{C_REPR_ALIAS}<{ty}>")
    } else {
        format!("<{ty} as ffier::FfiType<{tok}>>::CRepr")
    }
}

/// `<ty as IntoC>::into_c(arg)`.
pub(crate) fn into_c(ty: &str, tok: &str, arg: &str) -> String {
    format!("<{ty} as ffier::IntoC<{tok}>>::into_c({arg})")
}

/// `<ty as FromC>::from_c(arg)`; the caller supplies the `unsafe` block.
pub(crate) fn from_c(ty: &str, tok: &str, arg: &str) -> String {
    format!("<{ty} as ffier::FromC<{tok}>>::from_c({arg})")
}

/// Shorthands for a schema type, using its rendered Rust type.
impl TypeRefExt for TypeRef {
    fn c_repr_static(&self, lib: &Library) -> String {
        c_repr(&self.to_rust_type_static(), &token(self, lib))
    }
    fn convert_into_c(&self, lib: &Library, arg: &str) -> String {
        into_c(&self.to_rust_type(), &token(self, lib), arg)
    }
    fn convert_from_c(&self, lib: &Library, arg: &str) -> String {
        from_c(&self.to_rust_type(), &token(self, lib), arg)
    }
}

pub(crate) trait TypeRefExt {
    /// `CRepr` of the type with lifetimes erased to `'static`.
    fn c_repr_static(&self, lib: &Library) -> String;
    /// `<T as IntoC>::into_c(arg)`.
    fn convert_into_c(&self, lib: &Library, arg: &str) -> String;
    /// `<T as FromC>::from_c(arg)`; the caller supplies the `unsafe` block.
    fn convert_from_c(&self, lib: &Library, arg: &str) -> String;
}

/// Emit `FfiType` + `IntoC` + `FromC` impls of `ty` under this client's
/// token. `into_c` is the body of `fn into_c(self)`; `from_c` is the body
/// of `unsafe fn from_c(repr)`.
pub(crate) fn emit_conversions(
    out: &mut String,
    cfg: Option<&ffier_schema::Cfg>,
    ty: &str,
    repr: &str,
    into_c: &str,
    from_c: &str,
) {
    use std::fmt::Write;

    crate::cfg::emit_cfg_attr(out, cfg);
    writeln!(
        out,
        "impl ffier::FfiType<{TOKEN}> for {ty} {{ type CRepr = {repr}; }}"
    )
    .unwrap();
    crate::cfg::emit_cfg_attr(out, cfg);
    writeln!(out, "impl ffier::IntoC<{TOKEN}> for {ty} {{").unwrap();
    writeln!(out, "    fn into_c(self) -> {repr} {{ {into_c} }}").unwrap();
    writeln!(out, "}}").unwrap();
    crate::cfg::emit_cfg_attr(out, cfg);
    writeln!(out, "impl ffier::FromC<{TOKEN}> for {ty} {{").unwrap();
    writeln!(
        out,
        "    unsafe fn from_c(repr: {repr}) -> Self {{ {from_c} }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}
