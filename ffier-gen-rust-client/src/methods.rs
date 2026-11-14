//! Emit safe method wrappers and their FFI call bodies.

use std::fmt::Write;

use ffier_schema::{Library, Method, Receiver};

use crate::calls::{BuilderReceiver, ReturnPolicy, emit_call_body};
use crate::cfg::emit_cfg_attr;
use crate::signature::{
    build_wrapper_return_type, emit_doc_attrs, format_param_sig, format_receiver, lifetime_generics,
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

    emit_doc_attrs(out, &m.doc, "    ");

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

    let self_arg = match m.receiver {
        Receiver::None => None,
        Receiver::Value if is_builder_type => {
            Some("&mut __handle as *mut *mut core::ffi::c_void as *mut core::ffi::c_void")
        }
        Receiver::Value => Some("__handle"),
        Receiver::Ref | Receiver::Mut => Some("self.0"),
    };
    let builder = match m.receiver {
        Receiver::Mut => Some(BuilderReceiver::Mut),
        Receiver::Value => Some(BuilderReceiver::Value { has_lifetimes }),
        Receiver::None | Receiver::Ref => None,
    };
    let policy = ReturnPolicy {
        builder,
        own_borrowed_handles: true,
    };
    emit_call_body(
        out, ffi_name, self_arg, &m.params, &m.ret, "        ", lib, policy,
    );
}
