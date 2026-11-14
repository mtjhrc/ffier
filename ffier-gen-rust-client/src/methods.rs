//! Emit safe method wrappers and their FFI call bodies.

use std::fmt::Write;

use ffier_schema::{Library, Method, Receiver, Return};

use crate::abi::optional_value_out_type;
use crate::calls::{build_ffi_param_args, emit_param_pre_bindings};
use crate::cfg::emit_cfg_attr;
use crate::error::find_error_dispatch_fns;
use crate::signature::{
    borrowed_handle_return_type, build_wrapper_return_type, format_param_sig, format_receiver,
    lifetime_generics,
};

pub(super) fn emit_method_wrapper(
    out: &mut String,
    m: &Method,
    ffi_name: &str,
    is_builder_type: bool,
    has_lifetimes: bool,
    lib: &Library,
) {
    emit_cfg_attr(out, m.cfg.as_ref());

    // Doc comments — escape inner quotes to prevent broken string literals
    for doc in &m.doc {
        let escaped = doc.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(out, "    #[doc = \"{escaped}\"]").unwrap();
    }

    // Method-level lifetime generics
    let method_lt = lifetime_generics(&m.method_lifetimes);

    // Signature: receiver + params + return type
    let receiver_str = format_receiver(m);

    let param_sigs: Vec<String> = m.params.iter().map(format_param_sig).collect();
    let params_str = param_sigs.join(", ");

    let ret_type = build_wrapper_return_type(m, lib);

    if m.receiver_lifetime.is_some() {
        writeln!(out, "    #[allow(clippy::needless_lifetimes)]").unwrap();
    }
    writeln!(
        out,
        "    pub fn {}{method_lt}({receiver_str}{params_str}){ret_type} {{",
        m.name
    )
    .unwrap();

    // Method body
    emit_method_body(out, m, ffi_name, is_builder_type, has_lifetimes, lib);

    writeln!(out, "    }}").unwrap();
}

fn emit_method_body(
    out: &mut String,
    m: &Method,
    ffi_name: &str,
    is_builder_type: bool,
    has_lifetimes: bool,
    lib: &Library,
) {
    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    // Build FFI call arguments
    let by_value_self = m.receiver == Receiver::Value;

    // Handle extraction for by-value self
    if by_value_self && is_builder_type {
        writeln!(
            out,
            "        let mut __handle = {{ let this = std::mem::ManuallyDrop::new(self); this.0 }};"
        )
        .unwrap();
    } else if by_value_self {
        writeln!(
            out,
            "        let __handle = {{ let this = std::mem::ManuallyDrop::new(self); this.0 }};"
        )
        .unwrap();
    }

    emit_param_pre_bindings(out, &m.params, "        ");

    // Build the FFI call argument list
    let mut ffi_args = Vec::new();

    // Self arg
    match m.receiver {
        Receiver::None => {}
        Receiver::Value if is_builder_type => {
            ffi_args.push(
                "&mut __handle as *mut *mut core::ffi::c_void as *mut core::ffi::c_void"
                    .to_string(),
            );
        }
        Receiver::Value => {
            ffi_args.push("__handle".to_string());
        }
        _ => {
            ffi_args.push("self.0".to_string());
        }
    }

    ffi_args.extend(build_ffi_param_args(&m.params, lib));
    let args_str = ffi_args.join(", ");
    // Separator for appending extra out-params after args_str
    let sep = if args_str.is_empty() { "" } else { ", " };

    // Emit body based on return kind
    let is_ok_handle = matches!(
        &m.ret,
        Return::Result {
            c_convention: ffier_schema::CResultConvention::HandleOrNull,
            ..
        }
    );

    match &m.ret {
        Return::Value(_) if is_builder && m.receiver == Receiver::Mut => {
            // Builder &mut self → call, return self
            writeln!(out, "        unsafe {{ {ffi_name}({args_str}) }};").unwrap();
            writeln!(out, "        self").unwrap();
        }
        Return::Value(_) if is_builder && by_value_self => {
            // Builder by-value self → call, reconstruct Self from __handle
            writeln!(out, "        unsafe {{ {ffi_name}({args_str}) }};").unwrap();
            if has_lifetimes {
                writeln!(out, "        Self(__handle, std::marker::PhantomData)").unwrap();
            } else {
                writeln!(out, "        Self(__handle)").unwrap();
            }
        }
        Return::Void => {
            writeln!(out, "        unsafe {{ {ffi_name}({args_str}) }}").unwrap();
        }
        Return::ObjectArray { .. } => {
            // Object array: wrap raw FfierObjectArray in ForeignSlice.
            writeln!(
                out,
                "        ForeignSlice::from_raw(unsafe {{ {ffi_name}({args_str}) }})"
            )
            .unwrap();
        }
        Return::OptionalValue(tr) => {
            let ty_static = optional_value_out_type(tr);
            let ty = tr.to_rust_type();
            writeln!(
                out,
                "        let mut __out = std::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();"
            )
            .unwrap();
            writeln!(
                out,
                "        let __is_some = unsafe {{ {ffi_name}({args_str}{sep}__out.as_mut_ptr()) }};"
            )
            .unwrap();
            writeln!(out, "        if __is_some {{").unwrap();
            writeln!(
                out,
                "            Some(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }})"
            )
            .unwrap();
            writeln!(out, "        }} else {{").unwrap();
            writeln!(out, "            None").unwrap();
            writeln!(out, "        }}").unwrap();
        }
        Return::Value(tr) => {
            writeln!(
                out,
                "        let __raw = unsafe {{ {ffi_name}({args_str}) }};"
            )
            .unwrap();
            if borrowed_handle_return_type(&m.ret, lib).is_some() {
                // Borrowed handle: wrap raw pointer in owned struct directly.
                let bare_name = &tr.type_name;
                writeln!(
                    out,
                    "        unsafe {{ <{bare_name} as FfiHandle>::__from_raw(__raw) }}"
                )
                .unwrap();
            } else {
                let ty = tr.to_rust_type();
                writeln!(out, "        unsafe {{ <{ty} as FfiType>::from_c(__raw) }}").unwrap();
            }
        }
        Return::Result {
            ok: Some(_),
            err_type,
            ..
        } if is_builder && by_value_self => {
            // Builder Result<Self, E> by-value: __handle was updated by bridge
            writeln!(
                out,
                "        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(out, "        let __r = unsafe {{ {ffi_name}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
            if has_lifetimes {
                writeln!(out, "        if __r == 0 {{ Ok(Self(__handle, std::marker::PhantomData)) }} else {{ Err(unsafe {{ {err_type}::from_ffi(__r, __err) }}) }}").unwrap();
            } else {
                writeln!(out, "        if __r == 0 {{ Ok(Self(__handle)) }} else {{ Err(unsafe {{ {err_type}::from_ffi(__r, __err) }}) }}").unwrap();
            }
        }
        Return::Result {
            ok: Some(_),
            err_type,
            ..
        } if is_builder && m.receiver == Receiver::Mut => {
            // Builder Result<Self, E> &mut self
            writeln!(
                out,
                "        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(out, "        let __r = unsafe {{ {ffi_name}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
            writeln!(
                out,
                "        if __r == 0 {{ Ok(self) }} else {{ Err(unsafe {{ {err_type}::from_ffi(__r, __err) }}) }}"
            )
            .unwrap();
        }
        Return::Result {
            ok: None, err_type, ..
        } => {
            writeln!(
                out,
                "        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(out, "        let __r = unsafe {{ {ffi_name}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
            writeln!(
                out,
                "        if __r == 0 {{ Ok(()) }} else {{ Err(unsafe {{ {err_type}::from_ffi(__r, __err) }}) }}"
            )
            .unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            ..
        } if is_ok_handle => {
            // Handle-or-null: returns handle, null on error
            let is_borrowed = borrowed_handle_return_type(&m.ret, lib).is_some();
            writeln!(
                out,
                "        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(out, "        let __raw = unsafe {{ {ffi_name}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
            let (error_result_fn, _error_destroy_fn) = find_error_dispatch_fns(lib);
            writeln!(out, "        if !__raw.is_null() {{").unwrap();
            if is_borrowed {
                let bare_name = &ok_tr.type_name;
                writeln!(
                    out,
                    "            Ok(unsafe {{ <{bare_name} as FfiHandle>::__from_raw(__raw) }})"
                )
                .unwrap();
            } else {
                let ty = ok_tr.to_rust_type();
                writeln!(
                    out,
                    "            Ok(unsafe {{ <{ty} as FfiType>::from_c(__raw) }})"
                )
                .unwrap();
            }
            writeln!(out, "        }} else {{").unwrap();
            writeln!(
                out,
                "            let __r = unsafe {{ {error_result_fn}(__err) }};"
            )
            .unwrap();
            writeln!(
                out,
                "            Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
            )
            .unwrap();
            writeln!(out, "        }}").unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            c_convention: ffier_schema::CResultConvention::OptionalValueOutParam,
        } => {
            // Presence flag plus a separate value out-param: `Ok(None)` leaves
            // the value buffer untouched, and errors write only `err_out`.
            let ty_static = optional_value_out_type(ok_tr);
            let mut bare = ok_tr.clone();
            bare.optional = false;
            let ty = bare.to_rust_type();
            writeln!(out, "        let mut __result_is_some = false;").unwrap();
            writeln!(
                out,
                "        let mut __out = std::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();"
            )
            .unwrap();
            writeln!(
                out,
                "        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(out, "        let __r = unsafe {{ {ffi_name}({args_str}{sep}&mut __result_is_some as *mut bool, __out.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
            writeln!(out, "        if __r == 0 {{").unwrap();
            writeln!(out, "            if __result_is_some {{").unwrap();
            writeln!(
                out,
                "                Ok(Some(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }}))"
            )
            .unwrap();
            writeln!(out, "            }} else {{").unwrap();
            writeln!(out, "                Ok(None)").unwrap();
            writeln!(out, "            }}").unwrap();
            writeln!(out, "        }} else {{").unwrap();
            writeln!(
                out,
                "            Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
            )
            .unwrap();
            writeln!(out, "        }}").unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            ..
        } => {
            // FfierResult-style with out-param
            writeln!(
                out,
                "        let mut __out = std::mem::MaybeUninit::uninit();"
            )
            .unwrap();
            writeln!(
                out,
                "        let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(out, "        let __r = unsafe {{ {ffi_name}({args_str}{sep}__out.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
            writeln!(out, "        if __r == 0 {{").unwrap();
            let ty = ok_tr.to_rust_type();
            writeln!(
                out,
                "            Ok(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }})"
            )
            .unwrap();
            writeln!(out, "        }} else {{").unwrap();
            writeln!(
                out,
                "            Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
            )
            .unwrap();
            writeln!(out, "        }}").unwrap();
        }
    }
}
