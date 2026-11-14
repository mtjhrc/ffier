//! Emit concrete trait implementations and cfg-specific default forwarders.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use ffier_schema::{Cfg, Library, Method, TraitImpl};

use crate::calls::{build_ffi_param_args, emit_ffi_call_return, emit_param_pre_bindings};
use crate::cfg::{self, emit_cfg_attr};
use crate::ffi::{ExternFn, SymbolInfo, emit_extern_fns, extern_fn_from_dispatch};
use crate::signature::build_trait_method_sig;
use crate::thread_safety::trait_thread_safety;

use super::definition::emit_callback_handle_methods;

/// Predicate under which a default forwarder must be emitted, or `None` when the
/// impl always provides its own body for this method.
///
/// An impl may override one cfg-exclusive variant of a method and inherit the
/// default for another, so a name-only check is wrong. Predicates are compared
/// after flattening single-element `all(...)`/`any(...)`, so `#[cfg(unix)]` and
/// `#[cfg(all(unix))]` count as the same variant; that is the only equivalence
/// this generator needs, since it sees both forms for one user-level cfg.
///
/// The forwarder is gated on the default's predicate minus every override guard,
/// so it exists exactly where the impl supplies no body.
pub(super) fn forwarder_cfg(dm: &Method, ti: &TraitImpl) -> Option<Option<Cfg>> {
    let overrides: Vec<&Method> = ti.methods.iter().filter(|m| m.name == dm.name).collect();
    if overrides.iter().any(|m| m.cfg.is_none()) {
        return None;
    }
    let override_cfgs: Vec<&Cfg> = overrides.iter().filter_map(|m| m.cfg.as_ref()).collect();
    if dm
        .cfg
        .as_ref()
        .is_some_and(|default| override_cfgs.iter().any(|o| cfg_eq(o, default)))
    {
        return None;
    }
    if override_cfgs.is_empty() {
        return Some(dm.cfg.clone());
    }
    // Negate the guard the override is actually emitted under, not the raw
    // predicate: `emit_cfg_attr` relaxes positive feature predicates for dynamic
    // lookup, and a raw negation would then overlap the relaxed guard.
    let mut guards: Vec<Cfg> = override_cfgs
        .iter()
        .map(|o| Cfg::Not(Box::new(cfg::client_cfg_cfg(o))))
        .collect();
    if let Some(default_cfg) = dm.cfg.as_ref() {
        guards.insert(0, default_cfg.clone());
    }
    Some(Some(Cfg::All(guards)))
}

/// Compare predicates, ignoring redundant single-element `all(...)`/`any(...)`
/// nesting.
fn cfg_eq(a: &Cfg, b: &Cfg) -> bool {
    normalize_cfg(a) == normalize_cfg(b)
}

/// Flatten single-element `all`/`any` so equivalent predicates compare equal.
fn normalize_cfg(cfg: &Cfg) -> Cfg {
    match cfg {
        Cfg::All(items) if items.len() == 1 => normalize_cfg(&items[0]),
        Cfg::Any(items) if items.len() == 1 => normalize_cfg(&items[0]),
        Cfg::Not(item) => Cfg::Not(Box::new(normalize_cfg(item))),
        Cfg::All(items) => Cfg::All(items.iter().map(normalize_cfg).collect()),
        Cfg::Any(items) => Cfg::Any(items.iter().map(normalize_cfg).collect()),
        other => other.clone(),
    }
}

pub(crate) fn emit_trait_impl(
    out: &mut String,
    ti: &TraitImpl,
    lib: &Library,
    defined_traits: &mut HashSet<String>,
    trait_defaults: &HashMap<String, Vec<&Method>>,
    symbols: &mut Vec<SymbolInfo>,
) {
    // Emit trait definition if not yet defined (trait-impl-only traits)
    if !defined_traits.contains(&ti.trait_name) {
        emit_simple_trait_def(out, ti, lib);
        defined_traits.insert(ti.trait_name.clone());
    }

    // Extern block
    let fns: Vec<ExternFn> = ti
        .methods
        .iter()
        .map(|m| extern_fn_from_dispatch(&m.ffi_name, m))
        .collect();
    emit_extern_fns(out, &fns, ti.cfg.as_ref(), symbols);

    // Build impl header with lifetimes
    let impl_generics = if !ti.lifetimes.is_empty() {
        format!(
            "<{}>",
            ti.lifetimes
                .iter()
                .map(|lt| format!("'{lt}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        String::new()
    };

    let trait_args = if !ti.trait_lifetime_args.is_empty() {
        let args: Vec<String> = ti
            .trait_lifetime_args
            .iter()
            .map(|lt| format!("'{lt}"))
            .collect();
        format!("<{}>", args.join(", "))
    } else {
        String::new()
    };

    let struct_args = if !ti.struct_lifetime_args.is_empty() {
        let args: Vec<String> = ti
            .struct_lifetime_args
            .iter()
            .map(|lt| format!("'{lt}"))
            .collect();
        format!("<{}>", args.join(", "))
    } else {
        String::new()
    };

    emit_cfg_attr(out, ti.cfg.as_ref());
    writeln!(
        out,
        "impl{impl_generics} {}{trait_args} for {}{struct_args} {{",
        ti.trait_name, ti.struct_name
    )
    .unwrap();

    // Emit methods from this impl
    for m in &ti.methods {
        emit_cfg_attr(out, m.cfg.as_ref());
        let sig = build_trait_method_sig(m, lib);
        writeln!(out, "    {sig} {{").unwrap();

        emit_param_pre_bindings(out, &m.params, "        ");
        let mut ffi_args = vec!["self.0".to_string()];
        ffi_args.extend(build_ffi_param_args(&m.params, lib));
        let args_str = ffi_args.join(", ");

        emit_ffi_call_return(out, &m.ffi_name, &args_str, &m.ret, "        ", lib);
        writeln!(out, "    }}").unwrap();
    }

    // Emit default method forwarders (from trait defaults not in this impl)
    if let Some(defaults) = trait_defaults.get(&ti.trait_name) {
        for dm in defaults {
            let Some(cfg) = forwarder_cfg(dm, ti) else {
                continue;
            };
            emit_cfg_attr(out, cfg.as_ref());
            let sig = build_trait_method_sig(dm, lib);
            writeln!(out, "    {sig} {{").unwrap();

            emit_param_pre_bindings(out, &dm.params, "        ");
            let mut ffi_args = vec!["self.0".to_string()];
            ffi_args.extend(build_ffi_param_args(&dm.params, lib));
            let args_str = ffi_args.join(", ");

            emit_ffi_call_return(out, &dm.ffi_name, &args_str, &dm.ret, "        ", lib);
            writeln!(out, "    }}").unwrap();
        }
    }

    // __into_raw_handle for known types (simple: extract raw pointer)
    writeln!(
        out,
        "    unsafe fn __into_raw_handle(self) -> *mut core::ffi::c_void {{"
    )
    .unwrap();
    writeln!(out, "        let this = std::mem::ManuallyDrop::new(self);").unwrap();
    writeln!(out, "        this.0").unwrap();
    writeln!(out, "    }}").unwrap();

    writeln!(
        out,
        "    fn __into_callback_handle(self) -> ffier::CallbackHandle<Box<Self>> where Self: Sized {{"
    )
    .unwrap();
    writeln!(
        out,
        "        unsafe fn destroy(raw: *mut core::ffi::c_void) {{"
    )
    .unwrap();
    writeln!(
        out,
        "            unsafe {{ drop(<{}{} as FfiType>::from_c(raw)) }};",
        ti.struct_name,
        if ti.struct_lifetime_args.is_empty() {
            String::new()
        } else {
            format!("<{}>", vec!["'_"; ti.struct_lifetime_args.len()].join(", "))
        }
    )
    .unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "        let this = std::mem::ManuallyDrop::new(self);").unwrap();
    writeln!(
        out,
        "        unsafe {{ ffier::CallbackHandle::from_raw(this.0, destroy) }}"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(
        out,
        "    fn __borrow_callback_handle(&self) -> ffier::CallbackHandle<&Self> {{"
    )
    .unwrap();
    writeln!(
        out,
        "        unsafe {{ ffier::CallbackHandle::from_raw(self.0, |_| {{}}) }}"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(
        out,
        "    fn __borrow_callback_handle_mut(&mut self) -> ffier::CallbackHandle<&mut Self> {{"
    )
    .unwrap();
    writeln!(
        out,
        "        unsafe {{ ffier::CallbackHandle::from_raw(self.0, |_| {{}}) }}"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn emit_simple_trait_def(out: &mut String, ti: &TraitImpl, lib: &Library) {
    // Build trait lifetime generics from trait_lifetime_args (excluding 'static)
    let trait_lt_params: Vec<String> = ti
        .trait_lifetime_args
        .iter()
        .filter(|lt| *lt != "static")
        .map(|lt| format!("'{lt}"))
        .collect();
    let trait_generics = if trait_lt_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", trait_lt_params.join(", "))
    };

    let cfg = lib
        .traits
        .iter()
        .find(|tr| tr.name == ti.trait_name)
        .and_then(|tr| tr.cfg.as_ref());
    let thread_safety = trait_thread_safety(lib, &ti.trait_name);
    let bounds = thread_safety.rust_bounds().collect::<Vec<_>>().join(" + ");
    let bounds = if bounds.is_empty() {
        String::new()
    } else {
        format!(": {bounds}")
    };
    emit_cfg_attr(out, cfg);
    writeln!(
        out,
        "pub trait {}{trait_generics}{bounds} {{",
        ti.trait_name
    )
    .unwrap();
    for m in &ti.methods {
        emit_cfg_attr(out, m.cfg.as_ref());
        let sig = build_trait_method_sig(m, lib);
        writeln!(out, "    {sig};").unwrap();
    }
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(out, "    /// # Safety").unwrap();
    writeln!(
        out,
        "    /// Callee must destroy this handle exactly once before borrowed data expires."
    )
    .unwrap();
    writeln!(
        out,
        "    unsafe fn __into_raw_handle(self) -> *mut core::ffi::c_void where Self: Sized;"
    )
    .unwrap();
    emit_callback_handle_methods(out, None, &lib.prefix, "");
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}
