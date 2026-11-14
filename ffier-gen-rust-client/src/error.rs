//! Error enums, payload accessors, and blessed error-trait dispatch declarations.

use std::fmt::Write;

use ffier_schema::{ErrorType, ErrorVariant, ErrorVariantData, ImplementableTrait, Library};

use crate::ffi::{SymbolInfo, emit_extern_fns, extern_fn, extern_fn_from_dispatch};

fn fieldless_variant_syntax(variant: &ErrorVariant) -> Option<String> {
    match &variant.data {
        ErrorVariantData::Fieldless => Some(variant.name.clone()),
        ErrorVariantData::Tuple(fields) if fields.is_empty() => Some(format!("{}()", variant.name)),
        ErrorVariantData::Tuple(_) => None,
    }
}

pub(crate) fn emit_error(out: &mut String, err: &ErrorType, lib: &Library) {
    let prefix = &lib.prefix;
    let error_type_tag = lib
        .type_entry(&err.name)
        .and_then(|entry| entry.type_tag)
        .expect("error type must have a type tag");
    let (_, error_destroy_fn) = find_error_dispatch_fns(lib);
    let payload_fn = format!("{prefix}_error_payload");

    // ErrorHandle — wraps the opaque handle, Drop calls ft_error_destroy
    let handle_name = format!("{}ErrorHandle", err.name);
    writeln!(out, "pub struct {handle_name}(*mut core::ffi::c_void);").unwrap();
    writeln!(out, "impl {handle_name} {{").unwrap();
    writeln!(
        out,
        "    fn handle(&self) -> *mut core::ffi::c_void {{ self.0 }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "impl Drop for {handle_name} {{").unwrap();
    writeln!(
        out,
        "    fn drop(&mut self) {{ if !self.0.is_null() {{ unsafe {{ {error_destroy_fn}(self.0) }} }} }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "impl std::fmt::Debug for {handle_name} {{").unwrap();
    writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ write!(f, \"ErrorHandle({{:?}})\", self.0) }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Per-variant data structs for data-carrying variants
    for v in &err.variants {
        if v.data.fields().is_empty() {
            continue;
        }
        let data_name = format!("{}{}Data", err.name, v.name);
        writeln!(out, "pub struct {data_name}({handle_name});").unwrap();
        writeln!(out, "impl {data_name} {{").unwrap();
        // Getter per field — calls ft_error_payload, returns borrowed data.
        // The payload borrows from the handle, so the getter returns a
        // reference tied to &self (which keeps the handle alive).
        for (i, type_ref) in v.data.typed_fields() {
            let getter_name = format!("field_{i}");
            // The field type in the schema is the owned type (e.g. Box<str>),
            // but the getter borrows — return &str for Box<str>, etc.
            let borrow_ref = type_ref.as_borrowed();
            let borrow_ty = borrow_ref.to_rust_type();
            let borrow_ty_static = borrow_ref.to_rust_type_static();
            writeln!(out, "    pub fn {getter_name}(&self) -> {borrow_ty} {{").unwrap();
            writeln!(
                out,
                "        let mut __buf = std::mem::MaybeUninit::<<{borrow_ty_static} as FfiType>::CRepr>::uninit();"
            )
            .unwrap();
            writeln!(
                out,
                "        unsafe {{ {payload_fn}(self.0.handle() as *const core::ffi::c_void, {i}, __buf.as_mut_ptr() as *mut core::ffi::c_void, core::mem::size_of::<<{borrow_ty_static} as FfiType>::CRepr>()) }};"
            )
            .unwrap();
            writeln!(
                out,
                "        unsafe {{ <{borrow_ty} as FfiType>::from_c(__buf.assume_init()) }}"
            )
            .unwrap();
            writeln!(out, "    }}").unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out, "impl std::fmt::Debug for {data_name} {{").unwrap();
        writeln!(
            out,
            "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ write!(f, \"{}(...)\") }}",
            v.name,
        )
        .unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    // Fieldless variants mirror the native enum. Payload variants retain
    // their native handle, including variants with opaque payloads.
    writeln!(out, "#[derive(Debug)]").unwrap();
    writeln!(out, "pub enum {} {{", err.name).unwrap();
    for v in &err.variants {
        if let Some(syntax) = fieldless_variant_syntax(v) {
            writeln!(out, "    {syntax},").unwrap();
        } else {
            let data_name = format!("{}{}Data", err.name, v.name);
            writeln!(out, "    {}({data_name}),", v.name).unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "impl {} {{", err.name).unwrap();
    if err.variants.iter().any(|v| v.data.fields().is_empty()) {
        writeln!(
            out,
            "    fn fieldless_handle(code: u32) -> {handle_name} {{"
        )
        .unwrap();
        writeln!(out, "        let raw = unsafe {{ {prefix}_error_new(ffier::ffier_result({error_type_tag}u32, code)) }};").unwrap();
        writeln!(
            out,
            "        assert!(!raw.is_null(), \"library cannot construct {} error code {{code}}\");",
            err.name
        )
        .unwrap();
        writeln!(out, "        {handle_name}(raw)").unwrap();
        writeln!(out, "    }}").unwrap();
    }
    // Native fieldless handles are released immediately on import.
    // Payload variants keep their handles for inspection and forwarding.
    writeln!(out, "    /// Import an owned native error handle.").unwrap();
    writeln!(out, "    ///").unwrap();
    writeln!(out, "    /// # Safety").unwrap();
    writeln!(
        out,
        "    /// The handle must be valid, non-null, and uniquely owned by the caller."
    )
    .unwrap();
    writeln!(
        out,
        "    /// It must contain the native error type represented by `Self` in the"
    )
    .unwrap();
    writeln!(out, "    /// library targeted by these bindings.").unwrap();
    writeln!(
        out,
        "    /// The type tag and variant code in `r` must match that native error."
    )
    .unwrap();
    writeln!(
        out,
        "    /// Ownership transfers to this function. Fieldless handles are destroyed"
    )
    .unwrap();
    writeln!(
        out,
        "    /// immediately; payload handles remain owned by the returned value."
    )
    .unwrap();
    writeln!(
        out,
        "    pub unsafe fn from_ffi(r: ffier::FfierResult, err_handle: *mut core::ffi::c_void) -> Self {{"
    )
    .unwrap();
    writeln!(out, "        assert!(r != ffier::FFIER_RESULT_SUCCESS, \"cannot import a successful FFI result as an error\");").unwrap();
    writeln!(out, "        let code = ffier::ffier_result_code(r);").unwrap();
    writeln!(out, "        let handle = {handle_name}(err_handle);").unwrap();
    writeln!(out, "        match code {{").unwrap();
    for v in &err.variants {
        if let Some(syntax) = fieldless_variant_syntax(v) {
            writeln!(
                out,
                "            {}u32 => {{ drop(handle); Self::{} }},",
                v.code, syntax
            )
            .unwrap();
        } else {
            let data_name = format!("{}{}Data", err.name, v.name);
            writeln!(
                out,
                "            {}u32 => Self::{}({data_name}(handle)),",
                v.code, v.name
            )
            .unwrap();
        }
    }
    writeln!(
        out,
        "            other => panic!(\"unknown {{}} error code {{}}\", \"{}\", other),",
        err.name
    )
    .unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();

    // Materialize fieldless values as native errors at the FFI boundary.
    // Payload errors transfer their existing native handle unchanged.
    writeln!(out, "    #[allow(dead_code)]").unwrap();
    writeln!(
        out,
        "    fn into_ffi(self) -> (ffier::FfierResult, *mut core::ffi::c_void) {{"
    )
    .unwrap();
    writeln!(out, "        let (code, handle) = match self {{").unwrap();
    for v in &err.variants {
        if let Some(syntax) = fieldless_variant_syntax(v) {
            writeln!(
                out,
                "            Self::{} => ({}u32, Self::fieldless_handle({}u32)),",
                syntax, v.code, v.code
            )
            .unwrap();
        } else {
            writeln!(
                out,
                "            Self::{}(data) => ({}u32, data.0),",
                v.name, v.code
            )
            .unwrap();
        }
    }
    writeln!(out, "        }};").unwrap();
    writeln!(out, "        let raw = handle.handle();").unwrap();
    writeln!(out, "        core::mem::forget(handle);").unwrap();
    writeln!(
        out,
        "        (ffier::ffier_result({error_type_tag}u32, code), raw)"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();

    // Status-only conversion, for callers that pass a null `err_out`. Deriving
    // the code from the variant avoids materializing a handle nobody would own.
    writeln!(out, "    #[allow(dead_code)]").unwrap();
    writeln!(out, "    fn ffi_result_of(&self) -> ffier::FfierResult {{").unwrap();
    writeln!(out, "        let code = match self {{").unwrap();
    for v in &err.variants {
        if let Some(syntax) = fieldless_variant_syntax(v) {
            writeln!(out, "            Self::{} => {}u32,", syntax, v.code).unwrap();
        } else {
            writeln!(out, "            Self::{}(_) => {}u32,", v.name, v.code).unwrap();
        }
    }
    writeln!(out, "        }};").unwrap();
    writeln!(
        out,
        "        ffier::ffier_result({error_type_tag}u32, code)"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Fieldless Display writes the static ABI message directly. Payload errors
    // keep native formatting through a temporary PushStr VtableHandle.
    writeln!(out, "impl std::fmt::Display for {} {{", err.name).unwrap();
    writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
    )
    .unwrap();
    let has_payload = err
        .variants
        .iter()
        .any(|variant| !variant.data.fields().is_empty());
    if has_payload {
        write!(out, "        let __error_handle = ").unwrap();
    } else {
        write!(out, "        ").unwrap();
    }
    writeln!(out, "match self {{").unwrap();
    for variant in &err.variants {
        if let Some(syntax) = fieldless_variant_syntax(variant) {
            writeln!(out, "            Self::{syntax} => {{").unwrap();
            writeln!(out, "                let message = unsafe {{ {prefix}_result_message(ffier::ffier_result({error_type_tag}u32, {}u32)) }};", variant.code).unwrap();
            if has_payload {
                writeln!(
                    out,
                    "                return f.write_str(unsafe {{ message.as_str_unchecked() }});"
                )
                .unwrap();
            } else {
                writeln!(
                    out,
                    "                f.write_str(unsafe {{ message.as_str_unchecked() }})"
                )
                .unwrap();
            }
            writeln!(out, "            }}").unwrap();
        } else {
            writeln!(
                out,
                "            Self::{}(data) => data.0.handle(),",
                variant.name
            )
            .unwrap();
        }
    }
    if has_payload {
        writeln!(out, "        }};").unwrap();
    } else {
        writeln!(out, "        }}").unwrap();
    }
    if has_payload {
        let push_str_info = find_push_str_trait(lib);
        let error_msg_fn = find_error_message_fn(lib);
        // FmtWriter: implements PushStr by writing to the Formatter
        writeln!(out, "        struct FmtWriter(*mut core::ffi::c_void);").unwrap();
        writeln!(out, "        impl PushStr for FmtWriter {{").unwrap();
        writeln!(
        out,
        "            fn push(&mut self, s: &str) -> bool {{ unsafe {{ (&mut *(self.0 as *mut std::fmt::Formatter<'_>)).write_str(s).is_ok() }} }}"
    )
    .unwrap();
        writeln!(out, "        }}").unwrap();
        writeln!(
        out,
        "        let mut __writer = FmtWriter(f as *mut std::fmt::Formatter<'_> as *mut core::ffi::c_void);"
    )
    .unwrap();
        writeln!(
            out,
            "        let __vtable: &'static {} = __ffier_vtable_PushStr::<FmtWriter>();",
            push_str_info.vtable_name
        )
        .unwrap();
        writeln!(out, "        let mut __temp = ffier::FfierHandle {{").unwrap();
        writeln!(out, "            type_tag: {}u32,", push_str_info.type_tag).unwrap();
        writeln!(out, "            metadata: 0,").unwrap();
        writeln!(out, "            value: ffier::VtableHandle {{").unwrap();
        writeln!(
            out,
            "                vtable_ptr: __vtable as *const {} as *const core::ffi::c_void,",
            push_str_info.vtable_name
        )
        .unwrap();
        writeln!(
        out,
        "                user_data: &mut __writer as *mut FmtWriter as *const core::ffi::c_void,"
    )
    .unwrap();
        writeln!(out, "            }},").unwrap();
        writeln!(out, "        }};").unwrap();
        writeln!(
        out,
        "        let __writer_handle = &mut __temp as *mut ffier::FfierHandle<ffier::VtableHandle> as *mut core::ffi::c_void;"
    )
    .unwrap();
        writeln!(
            out,
            "        unsafe {{ {error_msg_fn}(__error_handle, __writer_handle) }};"
        )
        .unwrap();
        writeln!(out, "        Ok(())").unwrap();
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // std::error::Error
    writeln!(out, "impl std::error::Error for {} {{}}", err.name).unwrap();
    writeln!(out).unwrap();
}

struct PushStrTraitInfo {
    type_tag: u32,
    vtable_name: String,
}

fn find_error_message_fn(lib: &Library) -> String {
    let (name, _) = lib
        .blessed(ffier_schema::Blessing::ErrorTrait)
        .expect("no type blessed as ErrorTrait found in schema");
    let error_trait = lib
        .traits
        .iter()
        .find(|t| t.name == name)
        .expect("blessed ErrorTrait type not found in traits list");
    error_trait
        .methods
        .iter()
        .find(|m| m.name == "message")
        .map(|m| m.ffi_name.clone())
        .expect("error trait has no 'message' method")
}

fn find_push_str_trait(lib: &Library) -> PushStrTraitInfo {
    let (name, _) = lib
        .blessed(ffier_schema::Blessing::PushStr)
        .expect("no type blessed as PushStr found in schema");
    let entry = lib.type_entry(name).unwrap();
    let type_tag = entry.type_tag.unwrap();
    let tr = lib
        .traits
        .iter()
        .find(|t| t.name == name)
        .expect("blessed PushStr type not found in traits list");
    PushStrTraitInfo {
        type_tag,
        vtable_name: tr.vtable_struct_name.clone(),
    }
}

/// Emit only the extern declarations for the error trait's dispatch functions.
/// This avoids emitting a `pub trait Error { ... }` that would collide with
/// the user's error enum, while still making the symbols available for
/// GLib-style Result wrappers.
pub(crate) fn emit_error_trait_externs(
    out: &mut String,
    tr: &ImplementableTrait,
    _lib: &Library,
    symbols: &mut Vec<SymbolInfo>,
) {
    let mut fns = Vec::new();
    for m in &tr.methods {
        fns.push(extern_fn_from_dispatch(&m.ffi_name, m));
    }
    // Destroy
    fns.push(extern_fn(
        &tr.destroy_ffi_name,
        vec!["handle: *mut core::ffi::c_void".to_string()],
        String::new(),
    ));
    emit_extern_fns(out, &fns, tr.cfg.as_ref(), symbols);
}

/// Look up the error trait's `result` dispatch ffi_name and `destroy` ffi_name
/// from the schema. The error trait is identified by `bless: ErrorTrait`.
pub(crate) fn find_error_dispatch_fns(lib: &Library) -> (&str, &str) {
    let (name, _) = lib
        .blessed(ffier_schema::Blessing::ErrorTrait)
        .expect("no type blessed as ErrorTrait found in schema");
    let error_trait = lib
        .traits
        .iter()
        .find(|t| t.name == name)
        .expect("blessed ErrorTrait type not found in traits list");
    let result_fn = error_trait
        .methods
        .iter()
        .find(|m| m.name == "result")
        .map(|m| m.ffi_name.as_str())
        .expect("error trait has no 'result' method");
    (result_fn, &error_trait.destroy_ffi_name)
}
