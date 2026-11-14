//! Emit traits that support callback vtables and default dispatch.

use std::fmt::Write;

use ffier_schema::{ImplementableTrait, Library, Method};

use crate::calls::{build_ffi_param_args, emit_ffi_call_return, emit_param_pre_bindings};
use crate::cfg::emit_cfg_attr;
use crate::ffi::{ExternFn, SymbolInfo, emit_extern_fns, extern_fn, extern_fn_from_dispatch};
use crate::signature::build_trait_method_sig;
use crate::thread_safety::emit_thread_safety_impls;

use super::vtable::{emit_vtable_constructor, emit_vtable_struct};

pub(crate) fn emit_implementable_trait(
    out: &mut String,
    tr: &ImplementableTrait,
    lib: &Library,
    symbols: &mut Vec<SymbolInfo>,
) {
    let entry = lib.type_entry(&tr.name).unwrap();
    let type_tag = entry.type_tag.unwrap();
    let vtable_name = &tr.vtable_struct_name;
    let wrapper_name = &tr.wrapper_name;
    // Trait definition
    let bounds = tr
        .thread_safety
        .rust_bounds()
        .collect::<Vec<_>>()
        .join(" + ");
    let bounds = if bounds.is_empty() {
        String::new()
    } else {
        format!(": {bounds}")
    };
    emit_cfg_attr(out, tr.cfg.as_ref());
    writeln!(out, "pub trait {}{bounds} {{", tr.name).unwrap();
    for m in &tr.methods {
        let Some(td) = &m.trait_definition else {
            continue;
        };

        emit_cfg_attr(out, m.cfg.as_ref());
        let method_sig = build_trait_method_sig(m, lib);

        if td.has_default {
            // Default impl via self-dispatch
            writeln!(out, "    {method_sig} where Self: Sized {{").unwrap();
            emit_default_dispatch_body(out, m, lib, tr, type_tag, td.index);
            writeln!(out, "    }}").unwrap();
        } else {
            writeln!(out, "    {method_sig};").unwrap();
        }
    }

    // Raw transfers cannot encode callback lifetimes.
    writeln!(out, "    /// # Safety").unwrap();
    writeln!(out, "    /// Callee must destroy this handle exactly once before any borrowed callback data expires.").unwrap();
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(
        out,
        "    unsafe fn __into_raw_handle(self) -> *mut core::ffi::c_void where Self: Sized {{"
    )
    .unwrap();
    writeln!(
        out,
        "        unsafe {{ self.__into_callback_handle().into_raw() }}"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();

    emit_callback_handle_methods(
        out,
        Some((type_tag, tr.name.as_str(), vtable_name)),
        &lib.prefix,
        &tr.destroy_ffi_name,
    );

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Vtable struct
    emit_vtable_struct(out, tr, lib, vtable_name);
    emit_cfg_attr(out, tr.cfg.as_ref());
    writeln!(out, "#[allow(non_snake_case)]").unwrap();
    writeln!(
        out,
        "fn __ffier_vtable_{}<__T: {}>() -> &'static {vtable_name} {{",
        tr.name, tr.name
    )
    .unwrap();
    emit_vtable_constructor(out, tr);
    writeln!(out, "}}").unwrap();

    // Self-dispatch externs for defaulted methods
    let default_fns: Vec<ExternFn> = tr
        .methods
        .iter()
        .filter_map(|m| {
            let td = m.trait_definition.as_ref()?;
            if !td.has_default {
                return None;
            }
            Some(extern_fn_from_dispatch(&m.ffi_name, m))
        })
        .collect();
    if !default_fns.is_empty() {
        emit_extern_fns(out, &default_fns, tr.cfg.as_ref(), symbols);
    }
    emit_extern_fns(
        out,
        &[extern_fn(
            &tr.destroy_ffi_name,
            vec!["handle: *mut core::ffi::c_void".to_string()],
            String::new(),
        )],
        tr.cfg.as_ref(),
        symbols,
    );

    // Wrapper struct
    emit_cfg_attr(out, tr.cfg.as_ref());
    writeln!(out, "pub struct {wrapper_name}<'callback>(*mut core::ffi::c_void, core::marker::PhantomData<&'callback mut ()>);").unwrap();
    emit_thread_safety_impls(
        out,
        wrapper_name,
        "<'callback>",
        tr.thread_safety,
        tr.cfg.as_ref(),
    );
    writeln!(out).unwrap();
    emit_cfg_attr(out, tr.cfg.as_ref());
    writeln!(out, "impl {wrapper_name}<'_> {{").unwrap();
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(out, "    /// # Safety").unwrap();
    writeln!(
        out,
        "    /// Callee must destroy this handle exactly once within its callback lifetime."
    )
    .unwrap();
    writeln!(out, "    pub unsafe fn __into_raw(self) -> *mut core::ffi::c_void {{ let this = std::mem::ManuallyDrop::new(self); this.0 }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    emit_cfg_attr(out, tr.cfg.as_ref());
    writeln!(out, "impl Drop for {wrapper_name}<'_> {{").unwrap();
    writeln!(
        out,
        "    fn drop(&mut self) {{ unsafe {{ {}(self.0) }} }}",
        tr.destroy_ffi_name
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Keep raw extraction unsafe; safe handles carry the value's borrow lifetime.
pub(super) fn emit_callback_handle_methods(
    out: &mut String,
    vtable: Option<(u32, &str, &str)>,
    prefix: &str,
    destroy_fn: &str,
) {
    writeln!(out, "    #[doc(hidden)]").unwrap();
    if let Some((type_tag, trait_name, vtable_name)) = vtable {
        writeln!(out, "    fn __into_callback_handle(self) -> ffier::CallbackHandle<Box<Self>> where Self: Sized {{").unwrap();
        writeln!(
            out,
            "        unsafe fn destroy(raw: *mut core::ffi::c_void) {{"
        )
        .unwrap();
        writeln!(out, "        unsafe {{ {destroy_fn}(raw) }};").unwrap();
        writeln!(out, "        }}").unwrap();
        writeln!(out, "        let value = ffier::VtableHandle {{ vtable_ptr: __ffier_vtable_{trait_name}::<Self>() as *const {vtable_name} as *const core::ffi::c_void, user_data: Box::into_raw(Box::new(self)) as *const core::ffi::c_void }};").unwrap();
        writeln!(out, "        unsafe {{ ffier::CallbackHandle::from_raw({prefix}_vtable_handle_new({type_tag}, value.vtable_ptr, value.user_data), destroy) }}").unwrap();
        writeln!(out, "    }}").unwrap();
    } else {
        writeln!(out, "    fn __into_callback_handle(self) -> ffier::CallbackHandle<Box<Self>> where Self: Sized;").unwrap();
    }
    for (method, reference) in [
        ("__borrow_callback_handle", "&self"),
        ("__borrow_callback_handle_mut", "&mut self"),
    ] {
        let borrowed = if reference == "&mut self" {
            "&mut Self"
        } else {
            "&Self"
        };
        writeln!(out, "    #[doc(hidden)]").unwrap();
        if let Some((type_tag, trait_name, vtable_name)) = vtable {
            let pointer = if reference == "&mut self" {
                "self as *mut Self"
            } else {
                "self as *const Self"
            };
            writeln!(
                out,
                "    fn {method}({reference}) -> ffier::CallbackHandle<{borrowed}> where Self: Sized {{"
            )
            .unwrap();
            writeln!(out, "        let value = ffier::VtableHandle {{ vtable_ptr: __ffier_vtable_{trait_name}::<Self>() as *const {vtable_name} as *const core::ffi::c_void, user_data: {pointer} as *const core::ffi::c_void }};").unwrap();
            // The borrowed shell belongs to this client, so allocate and free
            // it here. Its metadata forbids consuming it inside the library.
            // Never run the callback drop: user_data only borrows Self.
            writeln!(out, "        unsafe fn destroy(raw: *mut core::ffi::c_void) {{ unsafe {{ drop(Box::from_raw(raw as *mut ffier::FfierHandle<ffier::VtableHandle>)) }} }}").unwrap();
            writeln!(out, "        let shell = Box::new(ffier::FfierHandle {{ type_tag: {type_tag}, metadata: 0, value }});").unwrap();
            writeln!(out, "        unsafe {{ ffier::CallbackHandle::from_raw(Box::into_raw(shell) as *mut core::ffi::c_void, destroy) }}").unwrap();
            writeln!(out, "    }}").unwrap();
        } else {
            writeln!(
                out,
                "    fn {method}({reference}) -> ffier::CallbackHandle<{borrowed}> where Self: Sized;"
            )
            .unwrap();
        }
    }
}

pub(super) fn emit_default_dispatch_body(
    out: &mut String,
    m: &Method,
    lib: &Library,
    tr: &ImplementableTrait,
    type_tag: u32,
    index: usize,
) {
    let dispatch_fn = &m.ffi_name;
    let vtable_name = &tr.vtable_struct_name;

    writeln!(
        out,
        "        let __vtable: &'static {vtable_name} = __ffier_vtable_{}::<Self>();",
        tr.name
    )
    .unwrap();
    writeln!(
        out,
        "        let __metadata: u32 = ffier::default_dispatch_metadata({});",
        index
    )
    .unwrap();
    writeln!(out, "        let mut __temp = ffier::FfierHandle {{").unwrap();
    writeln!(out, "            type_tag: {type_tag}u32,").unwrap();
    writeln!(out, "            metadata: __metadata,").unwrap();
    writeln!(out, "            value: ffier::VtableHandle {{").unwrap();
    writeln!(
        out,
        "                vtable_ptr: __vtable as *const {vtable_name} as *const core::ffi::c_void,"
    )
    .unwrap();
    writeln!(
        out,
        "                user_data: self as *const Self as *const core::ffi::c_void,"
    )
    .unwrap();
    writeln!(out, "            }},").unwrap();
    writeln!(out, "        }};").unwrap();

    emit_param_pre_bindings(out, &m.params, "        ");

    let mut call_args = vec![
        "&mut __temp as *mut ffier::FfierHandle<ffier::VtableHandle> as *mut core::ffi::c_void"
            .to_string(),
    ];
    call_args.extend(build_ffi_param_args(&m.params, lib));
    let args_str = call_args.join(", ");

    emit_ffi_call_return(out, dispatch_fn, &args_str, &m.ret, "        ", lib);
}
