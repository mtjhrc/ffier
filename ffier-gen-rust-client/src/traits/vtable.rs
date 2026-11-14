//! Emit callback vtable layouts and Rust trampoline constructors.

use std::collections::HashMap;
use std::fmt::Write;

use ffier_schema::{ImplementableTrait, Library, Method, ParamType, Receiver, Return};

use crate::abi::{
    optional_value_out_type, push_extern_param_types, push_extern_params,
    push_return_and_out_param_types, push_return_and_out_params,
};
use crate::cfg::{emit_cfg_attr, uncovered_guard};

pub(super) fn emit_vtable_struct(
    out: &mut String,
    tr: &ImplementableTrait,
    _lib: &Library,
    vtable_name: &str,
) {
    emit_cfg_attr(out, tr.cfg.as_ref());
    writeln!(out, "#[repr(C)]").unwrap();
    writeln!(out, "pub struct {vtable_name} {{").unwrap();
    writeln!(
        out,
        "    /// Number of virtual method slots (excluding drop), including reserved gaps."
    )
    .unwrap();
    writeln!(out, "    pub _num_slots: u16,").unwrap();
    writeln!(out, "    /// Reserved for future use; initialize to zero.").unwrap();
    writeln!(out, "    pub _reserved1: u16,").unwrap();
    writeln!(out, "    /// Reserved for future use; initialize to zero.").unwrap();
    writeln!(out, "    pub _reserved2: u32,").unwrap();
    writeln!(
        out,
        "    pub drop: Option<unsafe extern \"C\" fn(*mut core::ffi::c_void)>,"
    )
    .unwrap();

    let mut methods_by_index: HashMap<usize, Vec<&Method>> = HashMap::new();
    for m in &tr.methods {
        if let Some(td) = &m.trait_definition {
            methods_by_index.entry(td.index).or_default().push(m);
        }
    }

    for slot in 0..tr.num_slots {
        let Some(variants) = methods_by_index.get(&slot) else {
            writeln!(
                out,
                "    pub __reserved_{slot}: Option<unsafe extern \"C\" fn()>,"
            )
            .unwrap();
            continue;
        };
        for m in variants {
            emit_cfg_attr(out, m.cfg.as_ref());
            let mut params = vec!["*mut core::ffi::c_void".to_string()];
            push_extern_param_types(&m.params, &mut params);
            let ret = push_return_and_out_param_types(&m.ret, &mut params);
            let params_str = params.join(", ");
            writeln!(
                out,
                "    pub {}: Option<unsafe extern \"C\" fn({params_str}){ret}>,",
                m.name
            )
            .unwrap();
        }
        // Keep the slot filled wherever no variant applies, so later slots keep
        // their offsets.
        if let Some(guard) = uncovered_guard(variants.iter().map(|m| m.cfg.as_ref())) {
            writeln!(out, "#[cfg({guard})]").unwrap();
            writeln!(
                out,
                "    pub __reserved_{slot}: Option<unsafe extern \"C\" fn()>,"
            )
            .unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_vtable_constructor(out: &mut String, tr: &ImplementableTrait) {
    let vtable_name = &tr.vtable_struct_name;
    let num_slots = u16::try_from(tr.num_slots).expect("num_slots exceeds u16::MAX");

    writeln!(out, "        &{vtable_name} {{").unwrap();
    writeln!(out, "            _num_slots: {num_slots},").unwrap();
    writeln!(out, "            _reserved1: 0,").unwrap();
    writeln!(out, "            _reserved2: 0,").unwrap();

    // drop trampoline
    writeln!(out, "            drop: Some({{").unwrap();
    writeln!(out, "                unsafe extern \"C\" fn __drop_trampoline<__T>(__ud: *mut core::ffi::c_void) {{").unwrap();
    writeln!(
        out,
        "                    unsafe {{ drop(Box::from_raw(__ud as *mut __T)) }};"
    )
    .unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "                __drop_trampoline::<__T>").unwrap();
    writeln!(out, "            }}),").unwrap();

    let mut methods_by_index: HashMap<usize, Vec<&Method>> = HashMap::new();
    for m in &tr.methods {
        if let Some(td) = &m.trait_definition {
            methods_by_index.entry(td.index).or_default().push(m);
        }
    }

    for slot in 0..tr.num_slots {
        let Some(variants) = methods_by_index.get(&slot) else {
            writeln!(out, "            __reserved_{slot}: None,").unwrap();
            continue;
        };
        for m in variants {
            emit_cfg_attr(out, m.cfg.as_ref());
            let is_mut = m.receiver == Receiver::Mut;
            let borrow = if is_mut { "&mut *" } else { "&*" };

            // Build trampoline params — same convention as vtable struct field
            let mut tramp_params = vec!["__ud: *mut core::ffi::c_void".to_string()];
            push_extern_params(&m.params, &mut tramp_params);
            let ret = push_return_and_out_params(&m.ret, &mut tramp_params, false);
            let tramp_params_str = tramp_params.join(", ");
            let method_lifetimes = m
                .method_lifetimes
                .iter()
                .map(|lt| format!("'{lt}, "))
                .collect::<String>();
            // The erased receiver pointer is borrowed for the lifetime required
            // by the Rust method, so its concrete value must outlive that borrow.
            let receiver_bound = m
                .receiver_lifetime
                .as_ref()
                .map(|lt| format!(" + '{lt}"))
                .unwrap_or_default();

            writeln!(out, "            {}: Some({{", m.name).unwrap();
            if !m.method_lifetimes.is_empty() {
                writeln!(
                    out,
                    "                #[allow(clippy::extra_unused_lifetimes)]"
                )
                .unwrap();
            }
            writeln!(
                out,
                "                unsafe extern \"C\" fn __trampoline<{method_lifetimes}__T: {}{receiver_bound}>(",
                tr.name
            )
            .unwrap();
            writeln!(out, "                    {tramp_params_str},").unwrap();
            writeln!(out, "                ){ret} {{").unwrap();

            let cast = if is_mut {
                "__ud as *mut __T"
            } else {
                "__ud as *const __T"
            };
            writeln!(
                out,
                "                    let __val = unsafe {{ {borrow}({cast}) }};"
            )
            .unwrap();

            // Build call args
            let mut call_args = Vec::new();
            for p in &m.params {
                match &p.param_type {
                    ParamType::Regular(tr) => {
                        let ty = tr.to_rust_type();
                        call_args.push(format!(
                            "unsafe {{ <{ty} as FfiType>::from_c({}) }}",
                            p.name
                        ));
                    }
                    ParamType::ImplTrait { .. } => {
                        call_args.push(p.name.clone());
                    }
                    ParamType::Slice { element, .. } => {
                        let slice_name = format!("__slice_{}", p.name);
                        if element.ref_kind != ffier_schema::RefKind::None {
                            // Ref elements: reconstruct Vec<&T> from C repr array.
                            let elem_ty = element.to_rust_type();
                            writeln!(out,
                                "                    let {slice_name}: Vec<{elem_ty}> = unsafe {{ core::slice::from_raw_parts({}, {}_len) }}.iter().map(|v| unsafe {{ <{elem_ty} as FfiType>::from_c(*v) }}).collect();",
                                p.name, p.name
                            ).unwrap();
                            call_args.push(format!("&{slice_name}"));
                        } else {
                            // Non-ref elements (primitives): from_raw_parts gives &[T] directly.
                            writeln!(out,
                                "                    let {slice_name} = if {}_len == 0 {{ &[] as &[{}] }} else {{ unsafe {{ core::slice::from_raw_parts({}, {}_len) }} }};",
                                p.name, element.to_rust_type(), p.name, p.name
                            ).unwrap();
                            call_args.push(slice_name);
                        }
                    }
                }
            }
            let call_str = call_args.join(", ");

            let binding = if matches!(m.ret, Return::Void) {
                ""
            } else {
                "let __result = "
            };
            writeln!(
                out,
                "                    {binding}__val.{}({call_str});",
                m.name
            )
            .unwrap();

            match &m.ret {
                Return::Void => {
                    // Nothing to convert
                }
                Return::ObjectArray { .. } => {
                    // ObjectArray returns from vtable methods: pass through raw
                    writeln!(out, "                    __result.raw").unwrap();
                }
                Return::OptionalValue(tr) => {
                    let ty_static = optional_value_out_type(tr);
                    let mut bare = tr.clone();
                    bare.optional = false;
                    let ty = bare.to_rust_type();
                    writeln!(out, "                    match __result {{").unwrap();
                    writeln!(out, "                        Some(__value) => {{").unwrap();
                    writeln!(out, "                            unsafe {{ result.write(<{ty} as FfiType>::into_c(__value)) }};").unwrap();
                    writeln!(out, "                            true").unwrap();
                    writeln!(out, "                        }}").unwrap();
                    writeln!(out, "                        None => false,").unwrap();
                    writeln!(out, "                    }}").unwrap();
                    let _ = ty_static;
                }
                Return::Value(tr) => {
                    let ty = tr.to_rust_type();
                    writeln!(
                        out,
                        "                    <{ty} as FfiType>::into_c(__result)"
                    )
                    .unwrap();
                }
                Return::Result {
                    ok, c_convention, ..
                } => {
                    use ffier_schema::CResultConvention;
                    writeln!(out, "                    match __result {{").unwrap();
                    match c_convention {
                        CResultConvention::HandleOrNull => {
                            match ok {
                                Some(ok_tr) => {
                                    let ty = ok_tr.to_rust_type();
                                    writeln!(out, "                        Ok(__ok) => <{ty} as FfiType>::into_c(__ok),").unwrap();
                                }
                                None => {
                                    writeln!(
                                        out,
                                        "                        Ok(()) => core::ptr::null_mut(),"
                                    )
                                    .unwrap();
                                }
                            }
                            writeln!(out, "                        Err(__e) => {{").unwrap();
                            writeln!(out, "                            if !err_out.is_null() {{")
                                .unwrap();
                            writeln!(
                                out,
                                "                                let (_, __handle) = __e.into_ffi();"
                            )
                            .unwrap();
                            writeln!(
                                out,
                                "                                unsafe {{ *err_out = __handle }};"
                            )
                            .unwrap();
                            writeln!(out, "                            }}").unwrap();
                            writeln!(out, "                            core::ptr::null_mut()")
                                .unwrap();
                            writeln!(out, "                        }}").unwrap();
                        }
                        CResultConvention::OutParam => {
                            match ok {
                                Some(ok_tr) => {
                                    let ty = ok_tr.to_rust_type();
                                    writeln!(out, "                        Ok(__ok) => {{")
                                        .unwrap();
                                    writeln!(out, "                            unsafe {{ result.write(<{ty} as FfiType>::into_c(__ok)) }};").unwrap();
                                    writeln!(
                                        out,
                                        "                            0 // FFIER_RESULT_SUCCESS"
                                    )
                                    .unwrap();
                                    writeln!(out, "                        }}").unwrap();
                                }
                                None => {
                                    writeln!(out, "                        Ok(()) => 0, // FFIER_RESULT_SUCCESS").unwrap();
                                }
                            }
                            writeln!(out, "                        Err(__e) => {{").unwrap();
                            writeln!(out, "                            if err_out.is_null() {{")
                                .unwrap();
                            writeln!(out, "                                __e.ffi_result_of()")
                                .unwrap();
                            writeln!(out, "                            }} else {{").unwrap();
                            writeln!(out, "                                let (__code, __handle) = __e.into_ffi();").unwrap();
                            writeln!(
                                out,
                                "                                unsafe {{ *err_out = __handle }};"
                            )
                            .unwrap();
                            writeln!(out, "                                __code").unwrap();
                            writeln!(out, "                            }}").unwrap();
                            writeln!(out, "                        }}").unwrap();
                        }
                        CResultConvention::OptionalValueOutParam => {
                            let ok_tr = ok
                                .as_ref()
                                .expect("OptionalValueOutParam requires an ok type");
                            let mut bare = ok_tr.clone();
                            bare.optional = false;
                            let ty = bare.to_rust_type();
                            writeln!(out, "                        Ok(Some(__ok)) => {{").unwrap();
                            writeln!(out, "                            unsafe {{").unwrap();
                            writeln!(
                                out,
                                "                                result_is_some.write(true);"
                            )
                            .unwrap();
                            writeln!(out, "                                result.write(<{ty} as FfiType>::into_c(__ok));").unwrap();
                            writeln!(out, "                            }}").unwrap();
                            writeln!(out, "                            0 // FFIER_RESULT_SUCCESS")
                                .unwrap();
                            writeln!(out, "                        }}").unwrap();
                            writeln!(out, "                        Ok(None) => {{").unwrap();
                            writeln!(out, "                            unsafe {{ result_is_some.write(false) }};").unwrap();
                            writeln!(out, "                            0 // FFIER_RESULT_SUCCESS")
                                .unwrap();
                            writeln!(out, "                        }}").unwrap();
                            writeln!(out, "                        Err(__e) => {{").unwrap();
                            writeln!(out, "                            if err_out.is_null() {{")
                                .unwrap();
                            writeln!(out, "                                __e.ffi_result_of()")
                                .unwrap();
                            writeln!(out, "                            }} else {{").unwrap();
                            writeln!(out, "                                let (__code, __handle) = __e.into_ffi();").unwrap();
                            writeln!(
                                out,
                                "                                unsafe {{ *err_out = __handle }};"
                            )
                            .unwrap();
                            writeln!(out, "                                __code").unwrap();
                            writeln!(out, "                            }}").unwrap();
                            writeln!(out, "                        }}").unwrap();
                        }
                    }
                    writeln!(out, "                    }}").unwrap();
                }
            }

            writeln!(out, "                }}").unwrap();
            writeln!(out, "                __trampoline::<__T>").unwrap();
            writeln!(out, "            }}),").unwrap();
        }
        // Keep the slot filled wherever no variant applies, so later slots keep
        // their offsets.
        if let Some(guard) = uncovered_guard(variants.iter().map(|m| m.cfg.as_ref())) {
            writeln!(out, "#[cfg({guard})]").unwrap();
            writeln!(out, "            __reserved_{slot}: None,").unwrap();
        }
    }

    writeln!(out, "        }}").unwrap();
}
