//! Opaque handles, lifetime markers, and safe inherent methods.

use std::fmt::Write;

use ffier_schema::{ExportedType, Library, LifetimeVariance, Receiver, Return};

use crate::cfg::emit_cfg_attr;
use crate::ffi::{SymbolInfo, emit_extern_fns, extern_fn, extern_fn_from_method};
use crate::ffi_traits::TOKEN;
use crate::methods::emit_method_wrapper;
use crate::thread_safety::emit_thread_safety_impls;

pub(crate) fn emit_exported_type(
    out: &mut String,
    ty: &ExportedType,
    lib: &Library,
    symbols: &mut Vec<SymbolInfo>,
) {
    let entry = lib.type_entry(&ty.name).unwrap();
    let type_tag = entry.type_tag.unwrap();
    let lifetimes = &entry.lifetime_params;
    let has_lifetimes = !lifetimes.is_empty();
    // Lifetime generic strings
    let lt_params = if has_lifetimes {
        format!(
            "<{}>",
            lifetimes
                .iter()
                .map(|lt| format!("'{}", lt.name))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        String::new()
    };

    // Extern block: destroy + all methods
    let mut fns = vec![extern_fn(
        &ty.destroy_ffi_name,
        vec!["handle: *mut core::ffi::c_void".to_string()],
        String::new(),
    )];
    for m in &ty.methods {
        fns.push(extern_fn_from_method(&m.ffi_name, m, lib));
    }
    emit_extern_fns(out, &fns, ty.cfg.as_ref(), symbols);

    // Struct definition
    emit_cfg_attr(out, ty.cfg.as_ref());
    if has_lifetimes {
        if lifetimes.len() > 1 {
            writeln!(out, "#[allow(clippy::type_complexity)]").unwrap();
        }
        let markers: Vec<_> = lifetimes
            .iter()
            .map(|lt| {
                let variance = lt
                    .variance
                    .expect("opaque lifetime requires explicit variance in schema");
                match variance {
                    LifetimeVariance::Covariant => format!("&'{} ()", lt.name),
                    LifetimeVariance::Contravariant => format!("fn(&'{} ())", lt.name),
                    LifetimeVariance::Invariant => format!("*mut &'{} ()", lt.name),
                }
            })
            .collect();
        let phantom = if markers.len() == 1 {
            format!("std::marker::PhantomData<{}>", markers[0])
        } else {
            format!("std::marker::PhantomData<({})>", markers.join(", "))
        };
        writeln!(
            out,
            "pub struct {}{lt_params}(*mut core::ffi::c_void, {phantom});",
            ty.name
        )
        .unwrap();
    } else {
        writeln!(out, "pub struct {}(*mut core::ffi::c_void);", ty.name).unwrap();
    }
    writeln!(out).unwrap();

    emit_thread_safety_impls(out, &ty.name, &lt_params, ty.thread_safety, ty.cfg.as_ref());

    // The handle traits: the wrapper owns the handle pointer it holds.
    let cfg = ty.cfg.as_ref();
    let self_ty = format!("{}{lt_params}", ty.name);
    let c_handle_name = lib.c_type_of(&ty.name);
    emit_cfg_attr(out, cfg);
    writeln!(
        out,
        "impl{lt_params} ffier::FfiHandle<{TOKEN}> for {self_ty} {{"
    )
    .unwrap();
    writeln!(
        out,
        "    const C_HANDLE_NAME: &'static str = \"{c_handle_name}\";"
    )
    .unwrap();
    writeln!(out, "    const TYPE_TAG: u32 = {type_tag}u32;").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    emit_cfg_attr(out, cfg);
    writeln!(
        out,
        "impl{lt_params} ffier::ClientHandle<{TOKEN}> for {self_ty} {{"
    )
    .unwrap();
    writeln!(
        out,
        "    fn as_handle(&self) -> *mut core::ffi::c_void {{ self.0 }}"
    )
    .unwrap();
    writeln!(
        out,
        "    fn into_raw(self) -> *mut core::ffi::c_void {{ std::mem::ManuallyDrop::new(self).0 }}"
    )
    .unwrap();
    writeln!(out, "    /// # Safety").unwrap();
    writeln!(
        out,
        "    /// Handle must be valid, uniquely owned, and valid for every lifetime of Self."
    )
    .unwrap();
    if has_lifetimes {
        writeln!(
            out,
            "    /// Underlying value must uphold Self's declared lifetime variance."
        )
        .unwrap();
    }
    if !ty.thread_safety.is_empty() {
        writeln!(
            out,
            "    /// Underlying value must uphold this type's verified Send/Sync guarantees."
        )
        .unwrap();
    }
    let from_raw = if has_lifetimes {
        "Self(handle, std::marker::PhantomData)"
    } else {
        "Self(handle)"
    };
    writeln!(
        out,
        "    unsafe fn from_raw(handle: *mut core::ffi::c_void) -> Self {{ {from_raw} }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // By value, the wrapper hands its handle over (and takes one back).
    emit_cfg_attr(out, cfg);
    writeln!(
        out,
        "impl{lt_params} ffier::FfiType<{TOKEN}> for {self_ty} {{"
    )
    .unwrap();
    writeln!(out, "    type CRepr = *mut core::ffi::c_void;").unwrap();
    writeln!(out, "    const IS_HANDLE: bool = true;").unwrap();
    writeln!(out, "}}").unwrap();
    emit_cfg_attr(out, cfg);
    writeln!(
        out,
        "impl{lt_params} ffier::IntoC<{TOKEN}> for {self_ty} {{"
    )
    .unwrap();
    writeln!(
        out,
        "    fn into_c(self) -> *mut core::ffi::c_void {{ ffier::ClientHandle::<{TOKEN}>::into_raw(self) }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    emit_cfg_attr(out, cfg);
    writeln!(
        out,
        "impl{lt_params} ffier::FromC<{TOKEN}> for {self_ty} {{"
    )
    .unwrap();
    writeln!(
        out,
        "    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {{ unsafe {{ <Self as ffier::ClientHandle<{TOKEN}>>::from_raw(repr) }} }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Debug
    emit_cfg_attr(out, ty.cfg.as_ref());
    writeln!(
        out,
        "impl{lt_params} std::fmt::Debug for {}{lt_params} {{",
        ty.name
    )
    .unwrap();
    writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
    )
    .unwrap();
    writeln!(
        out,
        "        f.debug_tuple(\"{}\").field(&self.0).finish()",
        ty.name
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Methods
    emit_cfg_attr(out, ty.cfg.as_ref());
    writeln!(out, "impl{lt_params} {}{lt_params} {{", ty.name).unwrap();
    for m in &ty.methods {
        emit_method_wrapper(
            out,
            m,
            &m.ffi_name,
            ty.replaces_self_by_value,
            has_lifetimes,
            lib,
        );
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Default — emit if there's a zero-param `new()` static constructor returning Self
    let default_new = ty.methods.iter().find(|m| {
        m.name == "new"
            && m.receiver == Receiver::None
            && m.params.is_empty()
            && matches!(&m.ret, Return::Value(tr) if tr.type_name == ty.name)
    });
    if !has_lifetimes && let Some(new) = default_new {
        emit_cfg_attr(out, ty.cfg.as_ref());
        emit_cfg_attr(out, new.cfg.as_ref());
        writeln!(
            out,
            "impl Default for {} {{ fn default() -> Self {{ Self::new() }} }}",
            ty.name
        )
        .unwrap();
        writeln!(out).unwrap();
    }

    // Drop
    emit_cfg_attr(out, ty.cfg.as_ref());
    writeln!(out, "impl{lt_params} Drop for {}{lt_params} {{", ty.name).unwrap();
    writeln!(out, "    fn drop(&mut self) {{").unwrap();
    writeln!(out, "        unsafe {{ {}(self.0) }}", ty.destroy_ffi_name).unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}
