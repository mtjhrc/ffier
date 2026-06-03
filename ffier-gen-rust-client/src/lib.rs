//! Rust client bindings generator from ffier JSON schema.
//!
//! Reads a `ffier-{prefix}.json` and produces a complete Rust source file with:
//! - Error enums with `from_ffi`, `Display`, `Error` impls
//! - Handle wrapper structs with extern "C" declarations
//! - Safe method wrappers with lifetime-correct signatures
//! - Trait definitions with vtable structs for implementable traits
//! - Trait impl blocks for concrete types

use ffier_schema::{
    EnumType, ErrorType, ExportedType, FreeFunction, ImplementableTrait, Library, Method,
    MethodContext, ParamType, Receiver, Return, TraitImpl, TypeKind,
};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// Generate complete Rust client source from a library schema.
pub fn generate(lib: &Library) -> String {
    let mut out = String::new();

    // Emit imports for blessed types that need special std imports.
    // Each blessing contributes individual import symbols; collected and
    // emitted as a single `use` statement to avoid duplicates.
    {
        use ffier_schema::Blessing;
        let blessing_symbols: &[(Blessing, &[&str])] = &[
            (Blessing::RawFd, &["RawFd"]),
            (Blessing::BorrowedFd, &["AsRawFd", "BorrowedFd", "RawFd"]),
            (Blessing::OwnedFd, &["FromRawFd", "OwnedFd", "RawFd"]),
        ];
        let mut symbols: Vec<&str> = Vec::new();
        for (blessing, syms) in blessing_symbols {
            if lib.blessed(*blessing).is_some() {
                for sym in *syms {
                    if !symbols.contains(sym) {
                        symbols.push(sym);
                    }
                }
            }
        }
        if !symbols.is_empty() {
            symbols.sort();
            let list = symbols.join(", ");
            writeln!(out, "use std::os::unix::io::{{{list}}};").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Emit local FfiHandle and FfiType traits (standalone, no ffier dependency for traits)
    writeln!(
        out,
        "/// Marker trait for types exported as opaque C handles."
    )
    .unwrap();
    writeln!(out, "pub trait FfiHandle {{").unwrap();
    writeln!(out, "    const C_HANDLE_NAME: &'static str;").unwrap();
    writeln!(out, "    const TYPE_TAG: u32;").unwrap();
    writeln!(
        out,
        "    unsafe fn as_handle(&self) -> *mut core::ffi::c_void;"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// Maps Rust types to C-compatible representations.").unwrap();
    writeln!(out, "pub trait FfiType {{").unwrap();
    writeln!(out, "    type CRepr;").unwrap();
    writeln!(out, "    const C_TYPE_NAME: &'static str;").unwrap();
    writeln!(out, "    const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> Self::CRepr;").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: Self::CRepr) -> Self;").unwrap();
    writeln!(out, "    fn borrow_as_c(&self) -> Self::CRepr;").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Primitive FfiType impls
    writeln!(out, "macro_rules! impl_ffi_identity {{").unwrap();
    writeln!(out, "    ($($t:ty => $n:expr),* $(,)?) => {{ $(").unwrap();
    writeln!(out, "        impl FfiType for $t {{").unwrap();
    writeln!(out, "            type CRepr = $t; const C_TYPE_NAME: &'static str = $n; const IS_HANDLE: bool = false;").unwrap();
    writeln!(
        out,
        "            fn into_c(self) -> Self {{ self }} unsafe fn from_c(r: Self) -> Self {{ r }} fn borrow_as_c(&self) -> Self {{ *self }}"
    )
    .unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    )* }};").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "impl_ffi_identity! {{").unwrap();
    writeln!(
        out,
        "    i8 => \"int8_t\", i16 => \"int16_t\", i32 => \"int32_t\", i64 => \"int64_t\","
    )
    .unwrap();
    writeln!(
        out,
        "    u8 => \"uint8_t\", u16 => \"uint16_t\", u32 => \"uint32_t\", u64 => \"uint64_t\","
    )
    .unwrap();
    writeln!(
        out,
        "    isize => \"ssize_t\", usize => \"size_t\", bool => \"bool\","
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // &str FfiType impl
    writeln!(out, "impl FfiType for &str {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierStr\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ unsafe {{ ffier::FfierBytes::from_str(self) }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ unsafe {{ let b = core::slice::from_raw_parts(repr.data, repr.len); core::str::from_utf8_unchecked(b) }} }}").unwrap();
    writeln!(out, "    fn borrow_as_c(&self) -> ffier::FfierBytes {{ unsafe {{ ffier::FfierBytes::from_str(self) }} }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Option<&str> FfiType impl
    writeln!(out, "impl<'a> FfiType for Option<&'a str> {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierStr\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ match self {{ Some(s) => unsafe {{ ffier::FfierBytes::from_str(s) }}, None => ffier::FfierBytes::EMPTY }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ if repr.data.is_null() {{ None }} else {{ unsafe {{ Some(core::str::from_utf8_unchecked(core::slice::from_raw_parts(repr.data, repr.len))) }} }} }}").unwrap();
    writeln!(out, "    fn borrow_as_c(&self) -> ffier::FfierBytes {{ match self {{ Some(s) => unsafe {{ ffier::FfierBytes::from_str(s) }}, None => ffier::FfierBytes::EMPTY }} }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Box<str> FfiType impl
    writeln!(out, "impl FfiType for Box<str> {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierStr\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ let leaked: &mut str = Box::leak(self); ffier::FfierBytes {{ data: leaked.as_mut_ptr() as *const u8, len: leaked.len() }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ unsafe {{ let slice = core::slice::from_raw_parts_mut(repr.data as *mut u8, repr.len); Box::from_raw(core::str::from_utf8_unchecked_mut(slice)) }} }}").unwrap();
    writeln!(out, "    fn borrow_as_c(&self) -> ffier::FfierBytes {{ ffier::FfierBytes {{ data: self.as_ptr(), len: self.len() }} }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // &[u8] FfiType impl
    writeln!(out, "impl FfiType for &[u8] {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierBytes\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ unsafe {{ ffier::FfierBytes::from_bytes(self) }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ unsafe {{ if repr.data.is_null() {{ &[] }} else {{ core::slice::from_raw_parts(repr.data, repr.len) }} }} }}").unwrap();
    writeln!(out, "    fn borrow_as_c(&self) -> ffier::FfierBytes {{ unsafe {{ ffier::FfierBytes::from_bytes(self) }} }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Blessed fd type FfiType impls
    emit_blessed_fd_impls(&mut out, lib);

    // Handle reference FfiType impls
    writeln!(out, "impl<T: FfiHandle + 'static> FfiType for &T {{").unwrap();
    writeln!(out, "    type CRepr = *mut core::ffi::c_void; const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME; const IS_HANDLE: bool = true;").unwrap();
    writeln!(
        out,
        "    fn into_c(self) -> *mut core::ffi::c_void {{ unsafe {{ self.as_handle() }} }}"
    )
    .unwrap();
    writeln!(out, "    unsafe fn from_c(_: *mut core::ffi::c_void) -> Self {{ unimplemented!(\"client-side &T from_c\") }}").unwrap();
    writeln!(
        out,
        "    fn borrow_as_c(&self) -> *mut core::ffi::c_void {{ unsafe {{ self.as_handle() }} }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "impl<T: FfiHandle + 'static> FfiType for &mut T {{").unwrap();
    writeln!(out, "    type CRepr = *mut core::ffi::c_void; const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME; const IS_HANDLE: bool = true;").unwrap();
    writeln!(
        out,
        "    fn into_c(self) -> *mut core::ffi::c_void {{ unsafe {{ self.as_handle() }} }}"
    )
    .unwrap();
    writeln!(out, "    unsafe fn from_c(_: *mut core::ffi::c_void) -> Self {{ unimplemented!(\"client-side &mut T from_c\") }}").unwrap();
    writeln!(
        out,
        "    fn borrow_as_c(&self) -> *mut core::ffi::c_void {{ unsafe {{ self.as_handle() }} }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // 0. Enum constants (FfiType impls + constant values)
    for en in &lib.enum_constants {
        emit_enum_type(&mut out, en, lib);
    }

    // 0b. Bitflags constants (bitflags! invocations + FfiType impls)
    for bf in &lib.bitflags_constants {
        emit_bitflags_type(&mut out, bf, lib);
    }

    // 1. Error enums
    for err in &lib.errors {
        emit_error(&mut out, err, lib);
    }

    // 2. Exported types
    for ty in &lib.exported_types {
        emit_exported_type(&mut out, ty, lib);
    }

    // 3. Implementable traits
    // Build trait defaults map: trait_name → vec of defaulted methods
    let mut trait_defaults: HashMap<String, Vec<&Method>> = HashMap::new();
    for tr in &lib.traits {
        let defaults: Vec<_> = tr
            .methods
            .iter()
            .filter(|m| {
                matches!(
                    &m.context,
                    MethodContext::Trait {
                        has_default: true,
                        ..
                    }
                )
            })
            .collect();
        if !defaults.is_empty() {
            trait_defaults.insert(tr.name.clone(), defaults);
        }
    }

    let mut defined_traits: HashSet<String> = HashSet::new();
    for tr in &lib.traits {
        if lib.type_entry(&tr.name).and_then(|e| e.bless)
            == Some(ffier_schema::Blessing::ErrorTrait)
        {
            // The error trait is an internal dispatch mechanism — don't emit
            // a trait definition (it would collide with the error enum).
            // Only emit extern declarations so the GLib-style Result wrapper
            // can call error_result / error_destroy.
            emit_error_trait_externs(&mut out, tr, lib);
            defined_traits.insert(tr.name.clone());
            continue;
        }
        emit_implementable_trait(&mut out, tr, lib);
        defined_traits.insert(tr.name.clone());
    }

    // 4. Trait impls — only emit for structs that exist as handle types
    // in the client (exported types + vtable wrappers).
    for ti in &lib.trait_impls {
        let is_handle = lib
            .type_entry(&ti.struct_name)
            .is_some_and(|e| matches!(e.kind, TypeKind::Handle { .. }));
        if !is_handle {
            continue;
        }
        emit_trait_impl(&mut out, ti, lib, &mut defined_traits, &trait_defaults);
    }

    // 5. Free functions
    for f in &lib.free_functions {
        emit_free_function(&mut out, f, lib);
    }

    out
}

/// Generate from a JSON file path.
pub fn generate_from_file(json_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let json = std::fs::read_to_string(json_path)?;
    let lib = Library::from_json(&json)?;
    Ok(generate(&lib))
}

// ===========================================================================
// Error generation
// ===========================================================================

fn emit_error(out: &mut String, err: &ErrorType, lib: &Library) {
    let push_str_info = find_push_str_trait(lib);
    let prefix = &lib.prefix;
    let (_, error_destroy_fn) = find_error_dispatch_fns(lib);
    let payload_fn = format!("{prefix}_error_payload");

    // Find the error_message dispatch function from the Error trait
    let error_msg_fn = find_error_message_fn(lib);

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
        if v.fields.is_empty() {
            continue;
        }
        let data_name = format!("{}{}Data", err.name, v.name);
        writeln!(out, "pub struct {data_name}({handle_name});").unwrap();
        writeln!(out, "impl {data_name} {{").unwrap();
        // Getter per field — calls ft_error_payload, returns borrowed data.
        // The payload borrows from the handle, so the getter returns a
        // reference tied to &self (which keeps the handle alive).
        for (i, field) in v.fields.iter().enumerate() {
            let getter_name = format!("field_{i}");
            // The field type in the schema is the owned type (e.g. Box<str>),
            // but the getter borrows — return &str for Box<str>, etc.
            let borrow_ty = borrowed_type_for(&field.type_ref);
            writeln!(out, "    pub fn {getter_name}(&self) -> {borrow_ty} {{").unwrap();
            writeln!(
                out,
                "        let mut __buf = std::mem::MaybeUninit::<ffier::FfierBytes>::uninit();"
            )
            .unwrap();
            writeln!(
                out,
                "        unsafe {{ {payload_fn}(self.0.handle() as *const core::ffi::c_void, __buf.as_mut_ptr() as *mut core::ffi::c_void, core::mem::size_of::<ffier::FfierBytes>()) }};"
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

    // Enum definition — every variant holds ErrorHandle (fieldless) or
    // VariantData (data-carrying, which wraps ErrorHandle)
    writeln!(out, "#[derive(Debug)]").unwrap();
    writeln!(out, "pub enum {} {{", err.name).unwrap();
    for v in &err.variants {
        if v.fields.is_empty() {
            writeln!(out, "    {}({handle_name}),", v.name).unwrap();
        } else {
            let data_name = format!("{}{}Data", err.name, v.name);
            writeln!(out, "    {}({data_name}),", v.name).unwrap();
        }
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // from_ffi — just match on code and stash the handle
    writeln!(out, "impl {} {{", err.name).unwrap();
    writeln!(
        out,
        "    pub fn from_ffi(r: ffier::FfierResult, err_handle: *mut core::ffi::c_void) -> Self {{"
    )
    .unwrap();
    writeln!(out, "        let code = ffier::ffier_result_code(r);").unwrap();
    writeln!(out, "        let handle = {handle_name}(err_handle);").unwrap();
    writeln!(out, "        match code {{").unwrap();
    for v in &err.variants {
        if v.fields.is_empty() {
            writeln!(
                out,
                "            {}u32 => Self::{}(handle),",
                v.code, v.name
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

    // message() — calls ft_error_message with a PushStr handle wrapping a String
    writeln!(out, "    fn handle_ptr(&self) -> *mut core::ffi::c_void {{").unwrap();
    writeln!(out, "        match self {{").unwrap();
    for v in &err.variants {
        if v.fields.is_empty() {
            writeln!(out, "            Self::{}(h) => h.handle(),", v.name).unwrap();
        } else {
            writeln!(out, "            Self::{}(d) => d.0.handle(),", v.name).unwrap();
        }
    }
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Display — calls ft_error_message with a stack-local PushStr VtableHandle
    writeln!(out, "impl std::fmt::Display for {} {{", err.name).unwrap();
    writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
    )
    .unwrap();
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
        "        let __vtable: &'static {} = FmtWriter::__ffier_vtable();",
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
    writeln!(
        out,
        "                vtable_size: core::mem::size_of::<{}>() as u16,",
        push_str_info.vtable_name
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
        "        unsafe {{ {error_msg_fn}(self.handle_ptr(), __writer_handle) }};"
    )
    .unwrap();
    writeln!(out, "        Ok(())").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // std::error::Error
    writeln!(out, "impl std::error::Error for {} {{}}", err.name).unwrap();
    writeln!(out).unwrap();

    // Extern declaration for payload getter
    writeln!(out, "unsafe extern \"C\" {{").unwrap();
    writeln!(
        out,
        "    fn {payload_fn}(handle: *const core::ffi::c_void, out_buf: *mut core::ffi::c_void, buf_size: usize);"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
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
        .and_then(|m| match &m.context {
            MethodContext::Trait { ffi_name, .. } => Some(ffi_name.clone()),
            _ => None,
        })
        .expect("error trait has no 'message' method")
}

/// Check if a Return is a borrowed handle: `&HandleType` or `Result<&HandleType, E>`.
/// Returns the bare type name (e.g. "Gadget") if so.
fn borrowed_handle_return_type<'a>(ret: &'a Return, lib: &Library) -> Option<&'a str> {
    let tr = match ret {
        Return::Value(tr) => tr,
        Return::Result { ok: Some(tr), .. } => tr,
        _ => return None,
    };
    if tr.ref_kind != ffier_schema::RefKind::Shared && tr.ref_kind != ffier_schema::RefKind::Mut {
        return None;
    }
    lib.type_entry(&tr.type_name)
        .filter(|e| matches!(e.kind, TypeKind::Handle { .. }))
        .map(|_| tr.type_name.as_str())
}

/// Map an owned field type to its borrowed equivalent for error payload getters.
/// The getter borrows from the handle, so it can't return owned types.
fn borrowed_type_for(type_ref: &ffier_schema::TypeRef) -> String {
    if type_ref.owned {
        // Owned type (e.g. Box<str>) → shared reference (e.g. &str)
        let mut borrowed = type_ref.clone();
        borrowed.owned = false;
        borrowed.ref_kind = ffier_schema::RefKind::Shared;
        borrowed.to_rust_type()
    } else {
        type_ref.to_rust_type()
    }
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

// ===========================================================================
// Exported type generation
// ===========================================================================

fn emit_exported_type(out: &mut String, ty: &ExportedType, lib: &Library) {
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
                .map(|lt| format!("'{lt}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        String::new()
    };

    // Extern block: destroy + all methods
    writeln!(out, "unsafe extern \"C\" {{").unwrap();
    writeln!(
        out,
        "    pub fn {}(handle: *mut core::ffi::c_void);",
        ty.destroy_ffi_name
    )
    .unwrap();
    for m in &ty.methods {
        let MethodContext::Exportable { ffi_name } = &m.context else {
            continue;
        };
        let sig = build_extern_signature(ffi_name, m, lib);
        writeln!(out, "    pub fn {sig};").unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Struct definition
    if has_lifetimes {
        let phantom = if lifetimes.len() == 1 {
            format!("std::marker::PhantomData<&'{} ()>", lifetimes[0])
        } else {
            format!(
                "std::marker::PhantomData<({})>",
                lifetimes
                    .iter()
                    .map(|lt| format!("&'{lt} ()"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
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

    // __from_raw, __into_raw
    writeln!(out, "impl{lt_params} {}{lt_params} {{", ty.name).unwrap();
    writeln!(out, "    #[doc(hidden)]").unwrap();
    if has_lifetimes {
        writeln!(out, "    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {{ Self(ptr, std::marker::PhantomData) }}").unwrap();
    } else {
        writeln!(
            out,
            "    pub fn __from_raw(ptr: *mut core::ffi::c_void) -> Self {{ Self(ptr) }}"
        )
        .unwrap();
    }
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(out, "    pub fn __into_raw(self) -> *mut core::ffi::c_void {{ let this = std::mem::ManuallyDrop::new(self); this.0 }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiHandle
    writeln!(
        out,
        "impl{lt_params} FfiHandle for {}{lt_params} {{",
        ty.name
    )
    .unwrap();
    writeln!(
        out,
        "    const C_HANDLE_NAME: &'static str = \"{}\";",
        ty.name
    )
    .unwrap();
    writeln!(out, "    const TYPE_TAG: u32 = {type_tag}u32;").unwrap();
    writeln!(
        out,
        "    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {{ self.0 }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiType
    writeln!(out, "impl{lt_params} FfiType for {}{lt_params} {{", ty.name).unwrap();
    writeln!(out, "    type CRepr = *mut core::ffi::c_void;").unwrap();
    writeln!(
        out,
        "    const C_TYPE_NAME: &'static str = \"{}\";",
        ty.name
    )
    .unwrap();
    writeln!(
        out,
        "    fn into_c(self) -> *mut core::ffi::c_void {{ self.__into_raw() }}"
    )
    .unwrap();
    writeln!(
        out,
        "    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {{ Self::__from_raw(repr) }}"
    )
    .unwrap();
    writeln!(
        out,
        "    fn borrow_as_c(&self) -> *mut core::ffi::c_void {{ self.0 }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Debug
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
    writeln!(out, "impl{lt_params} {}{lt_params} {{", ty.name).unwrap();
    for m in &ty.methods {
        let MethodContext::Exportable { ffi_name } = &m.context else {
            continue;
        };
        emit_method_wrapper(out, m, ffi_name, ty.is_builder_type, has_lifetimes, lib);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Drop
    writeln!(out, "impl{lt_params} Drop for {}{lt_params} {{", ty.name).unwrap();
    writeln!(out, "    fn drop(&mut self) {{").unwrap();
    writeln!(out, "        unsafe {{ {}(self.0) }}", ty.destroy_ffi_name).unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ===========================================================================
// Extern signature building
// ===========================================================================

fn build_extern_signature(ffi_name: &str, m: &Method, lib: &Library) -> String {
    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    let mut params = Vec::new();

    // Self param — always *mut c_void in extern declarations.
    // Builder by-value self methods receive a single pointer; the caller
    // casts &mut handle_ptr to *mut *mut c_void → *mut c_void at the call site.
    match m.receiver {
        Receiver::None => {}
        _ => {
            params.push("handle: *mut core::ffi::c_void".to_string());
        }
    }

    // Method params
    push_extern_params(&m.params, &mut params);

    // Return type + extra out-params
    let ret_str = push_return_and_out_params(&m.ret, &mut params, is_builder);

    let params_str = params.join(", ");
    format!("{ffi_name}({params_str}){ret_str}")
}

// ===========================================================================
// Safe method wrapper
// ===========================================================================

fn emit_method_wrapper(
    out: &mut String,
    m: &Method,
    ffi_name: &str,
    is_builder_type: bool,
    has_lifetimes: bool,
    lib: &Library,
) {
    // Doc comments — escape inner quotes to prevent broken string literals
    for doc in &m.doc {
        let escaped = doc.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(out, "    #[doc = \"{escaped}\"]").unwrap();
    }

    // Method-level lifetime generics
    let method_lt = if !m.method_lifetimes.is_empty() {
        format!(
            "<{}>",
            m.method_lifetimes
                .iter()
                .map(|lt| format!("'{lt}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        String::new()
    };

    // Signature: receiver + params + return type
    let receiver_str = match m.receiver {
        Receiver::None => "",
        Receiver::Ref => "&self, ",
        Receiver::Mut => "&mut self, ",
        Receiver::Value => "self, ",
    };

    let param_sigs: Vec<String> = m.params.iter().map(|p| format_param_sig(p, lib)).collect();
    let params_str = param_sigs.join(", ");

    let ret_type = build_wrapper_return_type(m, lib);

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

fn build_wrapper_return_type(m: &Method, lib: &Library) -> String {
    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    if is_builder {
        return match m.receiver {
            Receiver::Mut => " -> &mut Self".to_string(),
            Receiver::Value => format!(" -> {}", m.ret.to_rust_type(&lib.type_registry)),
            _ => format!(" -> {}", m.ret.to_rust_type(&lib.type_registry)),
        };
    }

    // Borrowed handle returns (&HandleType) → return owned wrapper in client.
    // The wrapper's Drop calls destroy, which deallocates the borrowed shell
    // without dropping the inner value.
    if let Some(bare_name) = borrowed_handle_return_type(&m.ret, lib) {
        return match &m.ret {
            Return::Value(_) => format!(" -> {bare_name}"),
            Return::Result { err_type, .. } => format!(" -> Result<{bare_name}, {err_type}>"),
            _ => unreachable!(),
        };
    }

    match &m.ret {
        Return::Void => String::new(),
        Return::Value(_) | Return::Result { .. } => {
            format!(" -> {}", m.ret.to_rust_type(&lib.type_registry))
        }
    }
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

    emit_slice_pre_bindings(out, &m.params, "        ");

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
        Return::Value(tr) => {
            if borrowed_handle_return_type(&m.ret, lib).is_some() {
                // Borrowed handle: the FFI returns a *mut c_void handle.
                // Wrap in the owned struct — its Drop calls destroy, which
                // for borrowed handles just deallocates the shell.
                let bare_name = &tr.type_name;
                writeln!(
                    out,
                    "        let __raw = unsafe {{ {ffi_name}({args_str}) }};"
                )
                .unwrap();
                writeln!(out, "        {bare_name}(__raw)").unwrap();
            } else {
                let ty = tr.to_rust_type();
                writeln!(
                    out,
                    "        let __raw = unsafe {{ {ffi_name}({args_str}) }};"
                )
                .unwrap();
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
                writeln!(out, "        if __r == 0 {{ Ok(Self(__handle, std::marker::PhantomData)) }} else {{ Err({err_type}::from_ffi(__r, __err)) }}").unwrap();
            } else {
                writeln!(out, "        if __r == 0 {{ Ok(Self(__handle)) }} else {{ Err({err_type}::from_ffi(__r, __err)) }}").unwrap();
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
                "        if __r == 0 {{ Ok(self) }} else {{ Err({err_type}::from_ffi(__r, __err)) }}"
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
                "        if __r == 0 {{ Ok(()) }} else {{ Err({err_type}::from_ffi(__r, __err)) }}"
            )
            .unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            ..
        } if is_ok_handle => {
            // GLib-style: returns handle, null on error
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
                // Borrowed handle: wrap raw pointer in owned struct directly.
                let bare_name = &ok_tr.type_name;
                writeln!(out, "            Ok({bare_name}(__raw))").unwrap();
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
            writeln!(out, "            Err({err_type}::from_ffi(__r, __err))").unwrap();
            writeln!(out, "        }}").unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            ..
        } => {
            // FfierResult-style with out-param
            let ty = ok_tr.to_rust_type();
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
            writeln!(
                out,
                "            Ok(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }})"
            )
            .unwrap();
            writeln!(out, "        }} else {{").unwrap();
            writeln!(out, "            Err({err_type}::from_ffi(__r, __err))").unwrap();
            writeln!(out, "        }}").unwrap();
        }
    }
}

/// Emit only the extern declarations for the error trait's dispatch functions.
/// This avoids emitting a `pub trait Error { ... }` that would collide with
/// the user's error enum, while still making the symbols available for
/// GLib-style Result wrappers.
fn emit_error_trait_externs(out: &mut String, tr: &ImplementableTrait, _lib: &Library) {
    writeln!(out, "unsafe extern \"C\" {{").unwrap();
    for m in &tr.methods {
        let MethodContext::Trait { ffi_name, .. } = &m.context else {
            continue;
        };
        let sig = build_dispatch_extern_sig(ffi_name, m);
        writeln!(out, "    pub fn {sig};").unwrap();
    }
    // Destroy
    writeln!(
        out,
        "    pub fn {}(handle: *mut core::ffi::c_void);",
        tr.destroy_ffi_name
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Look up the error trait's `result` dispatch ffi_name and `destroy` ffi_name
/// from the schema. The error trait is identified by `bless: ErrorTrait`.
fn find_error_dispatch_fns(lib: &Library) -> (&str, &str) {
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
        .and_then(|m| match &m.context {
            MethodContext::Trait { ffi_name, .. } => Some(ffi_name.as_str()),
            _ => None,
        })
        .expect("error trait has no 'result' method");
    (result_fn, &error_trait.destroy_ffi_name)
}

// ===========================================================================
// Implementable trait generation
// ===========================================================================

fn emit_implementable_trait(out: &mut String, tr: &ImplementableTrait, lib: &Library) {
    let entry = lib.type_entry(&tr.name).unwrap();
    let type_tag = entry.type_tag.unwrap();
    let vtable_name = &tr.vtable_struct_name;
    let wrapper_name = &tr.wrapper_name;
    // Trait definition
    writeln!(out, "pub trait {} {{", tr.name).unwrap();
    for m in &tr.methods {
        let MethodContext::Trait {
            has_default, index, ..
        } = &m.context
        else {
            continue;
        };

        let method_sig = build_trait_method_sig(m, lib);

        if *has_default {
            // Default impl via self-dispatch
            writeln!(out, "    {method_sig} where Self: Sized {{").unwrap();
            emit_default_dispatch_body(out, m, lib, type_tag, vtable_name, *index);
            writeln!(out, "    }}").unwrap();
        } else {
            writeln!(out, "    {method_sig};").unwrap();
        }
    }

    // __ffier_vtable
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(
        out,
        "    fn __ffier_vtable() -> &'static {vtable_name} where Self: Sized {{"
    )
    .unwrap();
    emit_vtable_constructor(out, tr, lib);
    writeln!(out, "    }}").unwrap();

    // __into_raw_handle
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(
        out,
        "    fn __into_raw_handle(self) -> *mut core::ffi::c_void where Self: Sized {{"
    )
    .unwrap();
    writeln!(
        out,
        "        let __vtable: &'static {vtable_name} = Self::__ffier_vtable();"
    )
    .unwrap();
    writeln!(
        out,
        "        let __user_data = Box::into_raw(Box::new(self));"
    )
    .unwrap();
    writeln!(out, "        let vtable_size: u16 = core::mem::size_of::<{vtable_name}>().try_into().expect(\"vtable_size exceeds u16::MAX\");").unwrap();
    writeln!(
        out,
        "        ffier::ffier_handle_new_with_metadata({type_tag}u32, 0, ffier::VtableHandle {{"
    )
    .unwrap();
    writeln!(
        out,
        "            vtable_ptr: __vtable as *const {vtable_name} as *const core::ffi::c_void,"
    )
    .unwrap();
    writeln!(
        out,
        "            user_data: __user_data as *const core::ffi::c_void,"
    )
    .unwrap();
    writeln!(out, "            vtable_size,").unwrap();
    writeln!(out, "        }})").unwrap();
    writeln!(out, "    }}").unwrap();

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Vtable struct
    emit_vtable_struct(out, tr, lib, vtable_name);

    // Self-dispatch externs for defaulted methods
    let has_defaults = tr.methods.iter().any(|m| {
        matches!(
            &m.context,
            MethodContext::Trait {
                has_default: true,
                ..
            }
        )
    });
    if has_defaults {
        writeln!(out, "unsafe extern \"C\" {{").unwrap();
        for m in &tr.methods {
            let MethodContext::Trait {
                ffi_name,
                has_default,
                ..
            } = &m.context
            else {
                continue;
            };
            if !*has_default {
                continue;
            }
            let sig = build_dispatch_extern_sig(ffi_name, m);
            writeln!(out, "    pub fn {sig};").unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    // Wrapper struct
    writeln!(out, "pub struct {wrapper_name}(*mut core::ffi::c_void);").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "impl {wrapper_name} {{").unwrap();
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(out, "    pub fn __into_raw(self) -> *mut core::ffi::c_void {{ let this = std::mem::ManuallyDrop::new(self); this.0 }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "impl Drop for {wrapper_name} {{").unwrap();
    writeln!(out, "    fn drop(&mut self) {{}}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn build_trait_method_sig(m: &Method, lib: &Library) -> String {
    let receiver_str = match m.receiver {
        Receiver::None => "",
        Receiver::Ref => "&self, ",
        Receiver::Mut => "&mut self, ",
        Receiver::Value => "self, ",
    };

    let params: Vec<String> = m.params.iter().map(|p| format_param_sig(p, lib)).collect();
    let params_str = params.join(", ");

    let ret = match &m.ret {
        Return::Void => String::new(),
        _ => format!(" -> {}", m.ret.to_rust_type(&lib.type_registry)),
    };

    format!("fn {}({receiver_str}{params_str}){ret}", m.name)
}

fn emit_default_dispatch_body(
    out: &mut String,
    m: &Method,
    lib: &Library,
    type_tag: u32,
    vtable_name: &str,
    index: usize,
) {
    let MethodContext::Trait {
        ffi_name: dispatch_fn,
        ..
    } = &m.context
    else {
        panic!("emit_default_dispatch_body called on non-trait method");
    };

    writeln!(
        out,
        "        let __vtable: &'static {vtable_name} = Self::__ffier_vtable();"
    )
    .unwrap();
    writeln!(
        out,
        "        let __metadata: u32 = 2 | ({}u32 << 2);",
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
    writeln!(
        out,
        "                vtable_size: core::mem::size_of::<{vtable_name}>() as u16,"
    )
    .unwrap();
    writeln!(out, "            }},").unwrap();
    writeln!(out, "        }};").unwrap();

    emit_slice_pre_bindings(out, &m.params, "        ");

    let mut call_args = vec![
        "&mut __temp as *mut ffier::FfierHandle<ffier::VtableHandle> as *mut core::ffi::c_void"
            .to_string(),
    ];
    call_args.extend(build_ffi_param_args(&m.params, lib));
    let args_str = call_args.join(", ");

    emit_ffi_call_return(out, dispatch_fn, &args_str, &m.ret, "        ");
}

fn emit_vtable_struct(
    out: &mut String,
    tr: &ImplementableTrait,
    _lib: &Library,
    vtable_name: &str,
) {
    writeln!(out, "#[repr(C)]").unwrap();
    writeln!(out, "pub struct {vtable_name} {{").unwrap();
    writeln!(
        out,
        "    pub drop: Option<unsafe extern \"C\" fn(*mut core::ffi::c_void)>,"
    )
    .unwrap();

    let mut method_by_index: HashMap<usize, &Method> = HashMap::new();
    for m in &tr.methods {
        if let MethodContext::Trait { index, .. } = &m.context {
            method_by_index.insert(*index, m);
        }
    }

    for slot in 0..=tr.max_vtable_slot {
        if let Some(m) = method_by_index.get(&slot) {
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
        } else {
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

fn emit_vtable_constructor(out: &mut String, tr: &ImplementableTrait, lib: &Library) {
    let vtable_name = &tr.vtable_struct_name;

    writeln!(out, "        &{vtable_name} {{").unwrap();

    // drop trampoline
    writeln!(out, "            drop: Some({{").unwrap();
    writeln!(out, "                unsafe extern \"C\" fn __drop_trampoline<__T>(__ud: *mut core::ffi::c_void) {{").unwrap();
    writeln!(
        out,
        "                    unsafe {{ drop(Box::from_raw(__ud as *mut __T)) }};"
    )
    .unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "                __drop_trampoline::<Self>").unwrap();
    writeln!(out, "            }}),").unwrap();

    let mut method_by_index: HashMap<usize, &Method> = HashMap::new();
    for m in &tr.methods {
        if let MethodContext::Trait { index, .. } = &m.context {
            method_by_index.insert(*index, m);
        }
    }

    for slot in 0..=tr.max_vtable_slot {
        if let Some(m) = method_by_index.get(&slot) {
            let is_mut = m.receiver == Receiver::Mut;
            let borrow = if is_mut { "&mut *" } else { "&*" };

            // Build trampoline params — same convention as vtable struct field
            let mut tramp_params = vec!["__ud: *mut core::ffi::c_void".to_string()];
            push_extern_params(&m.params, &mut tramp_params);
            let ret = push_return_and_out_params(&m.ret, &mut tramp_params, false);
            let tramp_params_str = tramp_params.join(", ");

            writeln!(out, "            {}: Some({{", m.name).unwrap();
            writeln!(
                out,
                "                unsafe extern \"C\" fn __trampoline<__T: {}>(",
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
                    ParamType::Slice { .. } => {
                        let vec_name = format!("__slice_{}", p.name);
                        writeln!(out,
                            "                    let {vec_name}: Vec<&str> = unsafe {{ core::slice::from_raw_parts({}, {}_len) }}.iter().map(|b| unsafe {{ b.as_str_unchecked() }}).collect();",
                            p.name, p.name
                        ).unwrap();
                        call_args.push(format!("&{vec_name}"));
                    }
                }
            }
            let call_str = call_args.join(", ");

            writeln!(
                out,
                "                    let __result = __val.{}({call_str});",
                m.name
            )
            .unwrap();

            match &m.ret {
                Return::Void => {
                    // Nothing to convert
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
                    ok,
                    c_convention,
                    err_type,
                } => {
                    use ffier_schema::CResultConvention;
                    let err_type_tag = lib
                        .type_entry(err_type)
                        .and_then(|e| e.type_tag)
                        .unwrap_or(0);
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
                            writeln!(out, "                            unsafe {{ *err_out = Box::into_raw(Box::new(__e)) as *mut core::ffi::c_void }};").unwrap();
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
                            writeln!(out, "                            unsafe {{ *err_out = Box::into_raw(Box::new(__e)) as *mut core::ffi::c_void }};").unwrap();
                            // Any non-zero FfierResult signals error. The bridge reads
                            // the error handle from err_out, not the packed code.
                            writeln!(
                                out,
                                "                            ffier::ffier_result({err_type_tag}, 1)"
                            )
                            .unwrap();
                            writeln!(out, "                        }}").unwrap();
                        }
                    }
                    writeln!(out, "                    }}").unwrap();
                }
            }

            writeln!(out, "                }}").unwrap();
            writeln!(out, "                __trampoline::<Self>").unwrap();
            writeln!(out, "            }}),").unwrap();
        } else {
            writeln!(out, "            __reserved_{slot}: None,").unwrap();
        }
    }

    writeln!(out, "        }}").unwrap();
}

fn build_dispatch_extern_sig(extern_name: &str, m: &Method) -> String {
    let mut params = vec!["handle: *mut core::ffi::c_void".to_string()];
    push_extern_params(&m.params, &mut params);
    let ret = push_return_and_out_params(&m.ret, &mut params, false);
    let params_str = params.join(", ");
    format!("{extern_name}({params_str}){ret}")
}

// ===========================================================================
// Trait impl generation
// ===========================================================================

fn emit_trait_impl(
    out: &mut String,
    ti: &TraitImpl,
    lib: &Library,
    defined_traits: &mut HashSet<String>,
    trait_defaults: &HashMap<String, Vec<&Method>>,
) {
    // Emit trait definition if not yet defined (trait-impl-only traits)
    if !defined_traits.contains(&ti.trait_name) {
        emit_simple_trait_def(out, ti, lib);
        defined_traits.insert(ti.trait_name.clone());
    }

    // Extern block
    writeln!(out, "unsafe extern \"C\" {{").unwrap();
    for m in &ti.methods {
        let MethodContext::Trait { ffi_name, .. } = &m.context else {
            continue;
        };
        let sig = build_dispatch_extern_sig(ffi_name, m);
        writeln!(out, "    pub fn {sig};").unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

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

    writeln!(
        out,
        "impl{impl_generics} {}{trait_args} for {}{struct_args} {{",
        ti.trait_name, ti.struct_name
    )
    .unwrap();

    // Emit methods from this impl
    let impl_method_names: HashSet<&str> = ti.methods.iter().map(|m| m.name.as_str()).collect();

    for m in &ti.methods {
        let MethodContext::Trait { ffi_name, .. } = &m.context else {
            continue;
        };
        let sig = build_trait_method_sig(m, lib);
        writeln!(out, "    {sig} {{").unwrap();

        emit_slice_pre_bindings(out, &m.params, "        ");
        let mut ffi_args = vec!["self.0".to_string()];
        ffi_args.extend(build_ffi_param_args(&m.params, lib));
        let args_str = ffi_args.join(", ");

        emit_ffi_call_return(out, ffi_name, &args_str, &m.ret, "        ");
        writeln!(out, "    }}").unwrap();
    }

    // Emit default method forwarders (from trait defaults not in this impl)
    if let Some(defaults) = trait_defaults.get(&ti.trait_name) {
        for dm in defaults {
            if impl_method_names.contains(dm.name.as_str()) {
                continue;
            }

            let MethodContext::Trait {
                ffi_name: dispatch_fn,
                ..
            } = &dm.context
            else {
                continue;
            };
            let sig = build_trait_method_sig(dm, lib);
            writeln!(out, "    {sig} {{").unwrap();

            emit_slice_pre_bindings(out, &dm.params, "        ");
            let mut ffi_args = vec!["self.0".to_string()];
            ffi_args.extend(build_ffi_param_args(&dm.params, lib));
            let args_str = ffi_args.join(", ");

            emit_ffi_call_return(out, dispatch_fn, &args_str, &dm.ret, "        ");
            writeln!(out, "    }}").unwrap();
        }
    }

    // __into_raw_handle for known types (simple: extract raw pointer)
    writeln!(
        out,
        "    fn __into_raw_handle(self) -> *mut core::ffi::c_void {{"
    )
    .unwrap();
    writeln!(out, "        let this = std::mem::ManuallyDrop::new(self);").unwrap();
    writeln!(out, "        this.0").unwrap();
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

    writeln!(out, "pub trait {}{trait_generics} {{", ti.trait_name).unwrap();
    for m in &ti.methods {
        let sig = build_trait_method_sig(m, lib);
        writeln!(out, "    {sig};").unwrap();
    }
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(
        out,
        "    fn __into_raw_handle(self) -> *mut core::ffi::c_void where Self: Sized;"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Build FFI call arguments from method params. Each param is converted
/// via `FfiType::into_c`, `as_handle()`, `__into_raw_handle()`, or pre-bound
/// slice reference. For borrowed handle params (`&Handle`), uses `as_handle()`
/// instead of `into_c()` to avoid lifetime escape.
/// Append return-type out-params and return the return-type string for an
/// extern "C" function. Handles Void, Value, Result (FfierResult-style with
/// out-params and GLib-style for handle-returning Results).
///
/// Shared by all extern signature builders.
fn push_return_and_out_params(ret: &Return, params: &mut Vec<String>, is_builder: bool) -> String {
    use ffier_schema::CResultConvention;
    match ret {
        Return::Void => String::new(),
        Return::Value(_) if is_builder => String::new(),
        Return::Value(tr) => {
            let ty = tr.to_rust_type_static();
            format!(" -> <{ty} as FfiType>::CRepr")
        }
        Return::Result {
            ok, c_convention, ..
        } => match c_convention {
            CResultConvention::HandleOrNull => {
                params.push("err_out: *mut *mut core::ffi::c_void".to_string());
                " -> *mut core::ffi::c_void".to_string()
            }
            CResultConvention::OutParam => {
                if let Some(ok_tr) = ok.as_ref().filter(|_| !is_builder) {
                    let ty = ok_tr.to_rust_type_static();
                    params.push(format!("result: *mut <{ty} as FfiType>::CRepr"));
                }
                params.push("err_out: *mut *mut core::ffi::c_void".to_string());
                " -> ffier::FfierResult".to_string()
            }
        },
    }
}

/// Append C extern param strings (`"name: <Type as FfiType>::CRepr"` etc.)
/// for a list of schema params. Shared by all extern signature builders.
fn push_extern_params(params: &[ffier_schema::Param], out: &mut Vec<String>) {
    for p in params {
        match &p.param_type {
            ParamType::Regular(tr) => {
                let ty = tr.to_rust_type_static();
                out.push(format!("{}: <{ty} as FfiType>::CRepr", p.name));
            }
            ParamType::Slice { .. } => {
                out.push(format!("{}: *const ffier::FfierBytes", p.name));
                out.push(format!("{}_len: usize", p.name));
            }
            ParamType::ImplTrait { .. } => {
                out.push(format!("{}: *mut core::ffi::c_void", p.name));
            }
        }
    }
}

/// Append return type-only strings (no param names) for vtable fn pointer
/// fields. Companion to `push_extern_param_types`.
fn push_return_and_out_param_types(ret: &Return, out: &mut Vec<String>) -> String {
    use ffier_schema::CResultConvention;
    match ret {
        Return::Void => String::new(),
        Return::Value(tr) => {
            let ty = tr.to_rust_type_static();
            format!(" -> <{ty} as FfiType>::CRepr")
        }
        Return::Result {
            ok, c_convention, ..
        } => match c_convention {
            CResultConvention::HandleOrNull => {
                out.push("*mut *mut core::ffi::c_void".to_string());
                " -> *mut core::ffi::c_void".to_string()
            }
            CResultConvention::OutParam => {
                if let Some(ok_tr) = ok {
                    let ty = ok_tr.to_rust_type_static();
                    out.push(format!("*mut <{ty} as FfiType>::CRepr"));
                }
                out.push("*mut *mut core::ffi::c_void".to_string());
                " -> ffier::FfierResult".to_string()
            }
        },
    }
}

/// Append C type-only strings (no param names) for vtable struct fields.
fn push_extern_param_types(params: &[ffier_schema::Param], out: &mut Vec<String>) {
    for p in params {
        match &p.param_type {
            ParamType::Regular(tr) => {
                let ty = tr.to_rust_type_static();
                out.push(format!("<{ty} as FfiType>::CRepr"));
            }
            ParamType::Slice { .. } => {
                out.push("*const ffier::FfierBytes".to_string());
                out.push("usize".to_string());
            }
            ParamType::ImplTrait { .. } => {
                out.push("*mut core::ffi::c_void".to_string());
            }
        }
    }
}

fn build_ffi_param_args(params: &[ffier_schema::Param], lib: &Library) -> Vec<String> {
    let mut args = Vec::new();
    for p in params {
        match &p.param_type {
            ParamType::Regular(tr) => {
                // For borrowed references to handle types, use as_handle()
                // to avoid lifetime escape from into_c().
                let is_borrowed_handle = tr.ref_kind != ffier_schema::RefKind::None
                    && lib
                        .type_entry(&tr.type_name)
                        .is_some_and(|e| matches!(e.kind, ffier_schema::TypeKind::Handle { .. }));
                if is_borrowed_handle {
                    args.push(format!("FfiHandle::as_handle({})", p.name));
                } else {
                    let ty = tr.to_rust_type();
                    args.push(format!("<{ty} as FfiType>::into_c({})", p.name));
                }
            }
            ParamType::Slice { .. } => {
                args.push(format!("__ffi_{}.as_ptr()", p.name));
                args.push(format!("__ffi_{}.len()", p.name));
            }
            ParamType::ImplTrait { .. } => {
                args.push(format!("{}.__into_raw_handle()", p.name));
            }
        }
    }
    args
}

/// Emit slice pre-bindings (converting `&[&str]` to `Vec<FfierBytes>`)
/// for all slice params in the method.
fn emit_slice_pre_bindings(out: &mut String, params: &[ffier_schema::Param], indent: &str) {
    for p in params {
        if matches!(&p.param_type, ParamType::Slice { .. }) {
            writeln!(out, "{indent}let __ffi_{}: Vec<ffier::FfierBytes> = {}.iter().map(|s| unsafe {{ ffier::FfierBytes::from_str(s) }}).collect();",
                p.name, p.name).unwrap();
        }
    }
}

/// Emit an FFI call and handle the return value conversion.
fn emit_ffi_call_return(
    out: &mut String,
    ffi_name: &str,
    args_str: &str,
    ret: &Return,
    indent: &str,
) {
    let sep = if args_str.is_empty() { "" } else { ", " };
    match ret {
        Return::Void => {
            writeln!(out, "{indent}unsafe {{ {ffi_name}({args_str}) }}").unwrap();
        }
        Return::Value(tr) => {
            let ty = tr.to_rust_type();
            writeln!(
                out,
                "{indent}let __raw = unsafe {{ {ffi_name}({args_str}) }};"
            )
            .unwrap();
            writeln!(out, "{indent}unsafe {{ <{ty} as FfiType>::from_c(__raw) }}").unwrap();
        }
        Return::Result {
            ok: None, err_type, ..
        } => {
            writeln!(
                out,
                "{indent}let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(out, "{indent}let __r = unsafe {{ {ffi_name}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
            writeln!(
                out,
                "{indent}if __r == 0 {{ Ok(()) }} else {{ Err({err_type}::from_ffi(__r, __err)) }}"
            )
            .unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            c_convention,
        } => {
            use ffier_schema::CResultConvention;
            let ty = ok_tr.to_rust_type();
            writeln!(
                out,
                "{indent}let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            match c_convention {
                CResultConvention::HandleOrNull => {
                    writeln!(out, "{indent}let __raw = unsafe {{ {ffi_name}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
                    writeln!(out, "{indent}if !__raw.is_null() {{").unwrap();
                    writeln!(
                        out,
                        "{indent}    Ok(unsafe {{ <{ty} as FfiType>::from_c(__raw) }})"
                    )
                    .unwrap();
                    writeln!(out, "{indent}}} else {{").unwrap();
                    writeln!(out, "{indent}    Err({err_type}::from_ffi(0, __err))").unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                }
                CResultConvention::OutParam => {
                    let ty_static = ok_tr.to_rust_type_static();
                    writeln!(out, "{indent}let mut __result = core::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();").unwrap();
                    writeln!(out, "{indent}let __r = unsafe {{ {ffi_name}({args_str}{sep}__result.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
                    writeln!(out, "{indent}if __r == 0 {{").unwrap();
                    writeln!(out, "{indent}    Ok(unsafe {{ <{ty} as FfiType>::from_c(__result.assume_init()) }})").unwrap();
                    writeln!(out, "{indent}}} else {{").unwrap();
                    writeln!(out, "{indent}    Err({err_type}::from_ffi(__r, __err))").unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                }
            }
        }
    }
}

// ===========================================================================
// Blessed fd type generation
// ===========================================================================

fn emit_blessed_fd_impls(out: &mut String, lib: &Library) {
    use ffier_schema::Blessing;

    // Resolve the Rust repr type for fd aliases (e.g. "RawFd" → leaf "i32")
    fn fd_repr<'a>(entry: &'a ffier_schema::TypeEntry, lib: &'a Library) -> &'a str {
        match &entry.kind {
            ffier_schema::TypeKind::Alias { alias_of } => {
                // Walk one more level to get the Rust primitive name
                let leaf = lib
                    .type_entry(alias_of)
                    .unwrap_or_else(|| panic!("alias target `{alias_of}` not in registry"));
                match &leaf.kind {
                    ffier_schema::TypeKind::Primitive { .. } => alias_of.as_str(),
                    _ => panic!("fd alias target `{alias_of}` must be a Primitive"),
                }
            }
            _ => panic!("blessed fd type must be an Alias"),
        }
    }

    if let Some((_, entry)) = lib.blessed(Blessing::OwnedFd) {
        let repr = fd_repr(entry, lib);
        writeln!(out, "impl FfiType for OwnedFd {{").unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"int\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(out, "    fn into_c(self) -> {repr} {{ use std::os::unix::io::IntoRawFd; self.into_raw_fd() as {repr} }}").unwrap();
        writeln!(
            out,
            "    unsafe fn from_c(fd: {repr}) -> Self {{ unsafe {{ OwnedFd::from_raw_fd(fd as _) }} }}"
        )
        .unwrap();
        writeln!(
            out,
            "    fn borrow_as_c(&self) -> {repr} {{ self.as_raw_fd() as {repr} }}"
        )
        .unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    if let Some((_, entry)) = lib.blessed(Blessing::BorrowedFd) {
        let repr = fd_repr(entry, lib);
        writeln!(out, "impl<'fd> FfiType for BorrowedFd<'fd> {{").unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"int\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(
            out,
            "    fn into_c(self) -> {repr} {{ self.as_raw_fd() as {repr} }}"
        )
        .unwrap();
        writeln!(
            out,
            "    unsafe fn from_c(fd: {repr}) -> Self {{ unsafe {{ BorrowedFd::borrow_raw(fd as _) }} }}"
        )
        .unwrap();
        writeln!(
            out,
            "    fn borrow_as_c(&self) -> {repr} {{ self.as_raw_fd() as {repr} }}"
        )
        .unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Option<BorrowedFd>
        writeln!(out, "impl<'fd> FfiType for Option<BorrowedFd<'fd>> {{").unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"int\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(out, "    fn into_c(self) -> {repr} {{ match self {{ Some(fd) => fd.as_raw_fd() as {repr}, None => -1 }} }}").unwrap();
        writeln!(out, "    unsafe fn from_c(fd: {repr}) -> Self {{ if fd < 0 {{ None }} else {{ Some(unsafe {{ BorrowedFd::borrow_raw(fd as _) }}) }} }}").unwrap();
        writeln!(out, "    fn borrow_as_c(&self) -> {repr} {{ match self {{ Some(fd) => fd.as_raw_fd() as {repr}, None => -1 }} }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }
}

// ===========================================================================
// Enum type generation
// ===========================================================================

fn emit_enum_type(out: &mut String, en: &EnumType, lib: &Library) {
    let entry = lib.type_entry(&en.name).unwrap();
    let repr = match &entry.kind {
        TypeKind::Enum { alias_of } => alias_of.as_str(),
        _ => panic!("enum type must have TypeKind::Enum"),
    };

    // Enum definition
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]").unwrap();
    writeln!(out, "#[repr({repr})]").unwrap();
    writeln!(out, "pub enum {} {{", en.name).unwrap();
    for v in &en.variants {
        writeln!(out, "    {} = {},", v.name, v.value).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiType impl
    writeln!(out, "impl FfiType for {} {{", en.name).unwrap();
    writeln!(out, "    type CRepr = {repr};").unwrap();
    writeln!(
        out,
        "    const C_TYPE_NAME: &'static str = \"{}\";",
        en.name
    )
    .unwrap();
    writeln!(out, "    fn into_c(self) -> {repr} {{ self as {repr} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: {repr}) -> Self {{").unwrap();
    writeln!(out, "        match repr {{").unwrap();
    for v in &en.variants {
        writeln!(out, "            {} => Self::{},", v.value, v.name).unwrap();
    }
    writeln!(
        out,
        "            unknown => panic!(\"invalid {} discriminant: {{}}\", unknown),",
        en.name
    )
    .unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(
        out,
        "    fn borrow_as_c(&self) -> {repr} {{ *self as {repr} }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ===========================================================================
// Bitflags type generation
// ===========================================================================

fn emit_bitflags_type(out: &mut String, bf: &EnumType, lib: &Library) {
    let entry = lib.type_entry(&bf.name).unwrap();
    let repr = match &entry.kind {
        TypeKind::Bitflags { alias_of } => alias_of.as_str(),
        _ => panic!("bitflags type must have TypeKind::Bitflags"),
    };

    // bitflags! invocation
    writeln!(out, "bitflags::bitflags! {{").unwrap();
    writeln!(out, "    #[derive(Debug, Clone, Copy, PartialEq, Eq)]").unwrap();
    writeln!(out, "    pub struct {}: {repr} {{", bf.name).unwrap();
    for v in &bf.variants {
        writeln!(out, "        const {} = {};", v.name, v.value).unwrap();
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiType impl
    writeln!(out, "impl FfiType for {} {{", bf.name).unwrap();
    writeln!(out, "    type CRepr = {repr};").unwrap();
    writeln!(
        out,
        "    const C_TYPE_NAME: &'static str = \"{}\";",
        bf.name
    )
    .unwrap();
    writeln!(out, "    fn into_c(self) -> {repr} {{ self.bits() }}").unwrap();
    writeln!(
        out,
        "    unsafe fn from_c(repr: {repr}) -> Self {{ Self::from_bits_retain(repr) }}"
    )
    .unwrap();
    writeln!(out, "    fn borrow_as_c(&self) -> {repr} {{ self.bits() }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ===========================================================================
// Free function generation
// ===========================================================================

fn emit_free_function(out: &mut String, f: &FreeFunction, lib: &Library) {
    // Extern declaration
    writeln!(out, "unsafe extern \"C\" {{").unwrap();
    let sig = build_free_fn_extern_sig(&f.ffi_name, f);
    writeln!(out, "    pub fn {sig};").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Safe wrapper
    for doc in &f.doc {
        let escaped = doc.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(out, "#[doc = \"{escaped}\"]").unwrap();
    }

    let param_sigs: Vec<String> = f.params.iter().map(|p| format_param_sig(p, lib)).collect();
    let params_str = param_sigs.join(", ");

    let ret_type = match &f.ret {
        Return::Void => String::new(),
        Return::Value(tr) => format!(" -> {}", tr.to_rust_type()),
        Return::Result { ok, err_type, .. } => {
            let ok_str = match ok {
                Some(tr) => tr.to_rust_type(),
                None => "()".to_string(),
            };
            format!(" -> Result<{ok_str}, {err_type}>")
        }
    };

    writeln!(out, "pub fn {}({params_str}){ret_type} {{", f.name).unwrap();

    emit_slice_pre_bindings(out, &f.params, "    ");
    let ffi_args = build_ffi_param_args(&f.params, lib);
    let args_str = ffi_args.join(", ");
    let sep = if args_str.is_empty() { "" } else { ", " };

    match &f.ret {
        Return::Void => {
            writeln!(out, "    unsafe {{ {}({args_str}) }}", f.ffi_name).unwrap();
        }
        Return::Value(tr) => {
            let ty = tr.to_rust_type();
            writeln!(
                out,
                "    let __raw = unsafe {{ {}({args_str}) }};",
                f.ffi_name
            )
            .unwrap();
            writeln!(out, "    unsafe {{ <{ty} as FfiType>::from_c(__raw) }}").unwrap();
        }
        Return::Result {
            ok: None, err_type, ..
        } => {
            writeln!(
                out,
                "    let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            writeln!(
                out,
                "    let __r = unsafe {{ {}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};",
                f.ffi_name
            )
            .unwrap();
            writeln!(
                out,
                "    if __r == 0 {{ Ok(()) }} else {{ Err({err_type}::from_ffi(__r, __err)) }}"
            )
            .unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            c_convention,
        } => {
            let ty = ok_tr.to_rust_type();
            if *c_convention == ffier_schema::CResultConvention::HandleOrNull {
                writeln!(
                    out,
                    "    let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
                )
                .unwrap();
                writeln!(
                    out,
                    "    let __raw = unsafe {{ {}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};",
                    f.ffi_name
                )
                .unwrap();
                let (error_result_fn, _error_destroy_fn) = find_error_dispatch_fns(lib);
                writeln!(out, "    if !__raw.is_null() {{").unwrap();
                writeln!(
                    out,
                    "        Ok(unsafe {{ <{ty} as FfiType>::from_c(__raw) }})"
                )
                .unwrap();
                writeln!(out, "    }} else {{").unwrap();
                writeln!(
                    out,
                    "        let __r = unsafe {{ {error_result_fn}(__err) }};"
                )
                .unwrap();
                writeln!(out, "        Err({err_type}::from_ffi(__r, __err))").unwrap();
                writeln!(out, "    }}").unwrap();
            } else {
                writeln!(out, "    let mut __out = std::mem::MaybeUninit::uninit();").unwrap();
                writeln!(
                    out,
                    "    let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
                )
                .unwrap();
                writeln!(
                    out,
                    "    let __r = unsafe {{ {}({args_str}{sep}__out.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) }};",
                    f.ffi_name
                )
                .unwrap();
                writeln!(out, "    if __r == 0 {{").unwrap();
                writeln!(
                    out,
                    "        Ok(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }})"
                )
                .unwrap();
                writeln!(out, "    }} else {{").unwrap();
                writeln!(out, "        Err({err_type}::from_ffi(__r, __err))").unwrap();
                writeln!(out, "    }}").unwrap();
            }
        }
    }

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn build_free_fn_extern_sig(ffi_name: &str, f: &FreeFunction) -> String {
    let mut params = Vec::new();
    push_extern_params(&f.params, &mut params);
    let ret_str = push_return_and_out_params(&f.ret, &mut params, false);
    let params_str = params.join(", ");
    format!("{ffi_name}({params_str}){ret_str}")
}

fn format_param_sig(p: &ffier_schema::Param, _lib: &Library) -> String {
    match &p.param_type {
        ParamType::Regular(tr) => format!("{}: {}", p.name, tr.to_rust_type()),
        ParamType::Slice { .. } => format!("{}: &[&str]", p.name),
        ParamType::ImplTrait {
            trait_name,
            type_args,
            ..
        } => {
            if type_args.is_empty() {
                format!("{}: impl {trait_name}", p.name)
            } else {
                let args = type_args
                    .iter()
                    .map(|lt| format!("'{lt}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}: impl {trait_name}<{args}>", p.name)
            }
        }
    }
}
