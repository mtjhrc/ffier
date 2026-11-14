//! Coordinate complete Rust client generation from a library schema.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use ffier_schema::{Library, Method, TypeKind};

use crate::error::{emit_error, emit_error_trait_externs};
use crate::ffi::{emit_extern_fns, emit_symbol_api, extern_fn_private};
use crate::formatting::rustfmt;
use crate::functions::emit_free_function;
use crate::traits::{emit_implementable_trait, emit_trait_impl};
use crate::types::{
    emit_bitflags_type, emit_blessed_fd_impls, emit_enum_type, emit_exported_type,
    emit_value_structs,
};

/// Whether the library has a function returning `&[T]` / `&[&T]`, which the
/// bridge encodes as a `FfierObjectArray`. `ForeignSlice` and its free symbol
/// are emitted only then, so a library without such returns carries neither.
fn has_object_array_returns(lib: &Library) -> bool {
    let returns = lib
        .exported_types
        .iter()
        .flat_map(|ty| ty.methods.iter().map(|m| &m.ret))
        .chain(
            lib.traits
                .iter()
                .flat_map(|tr| tr.methods.iter().map(|m| &m.ret)),
        )
        .chain(lib.free_functions.iter().map(|f| &f.ret));
    returns
        .into_iter()
        .any(|ret| matches!(ret, ffier_schema::Return::ObjectArray { .. }))
}

/// Generate one Rust client source supporting both Unix and Windows linking modes.
/// The consuming crate selects the implementation with its `ffier-dynlookup` or
/// `ffier-strong` feature, one of which is required. `ffier-strong` takes
/// precedence when both are enabled.
pub fn generate(lib: &Library) -> String {
    let mut out = String::new();
    out.push_str("#[cfg(not(any(feature = \"ffier-dynlookup\", feature = \"ffier-strong\")))]\ncompile_error!(\"generated ffier clients require the `ffier-dynlookup` or `ffier-strong` feature\");\n\n");
    let object_array_returns = has_object_array_returns(lib);
    let handle_allocator = lib
        .traits
        .iter()
        .any(|tr| !tr.no_vtable)
        .then(|| format!("{}_vtable_handle_new", lib.prefix));

    // Emit imports for blessed types that need special std imports.
    // Each blessing contributes individual import symbols; collected and
    // emitted as a single `use` statement to avoid duplicates.
    {
        use ffier_schema::Blessing;
        // Generated signatures spell paths as bare `Path`, and the `&Path`
        // FfiType impl below is emitted for every library, so import it
        // unconditionally.
        writeln!(out, "use std::path::Path;").unwrap();
        let fd_blessing_symbols: &[(Blessing, &[&str])] = &[
            (Blessing::RawFd, &["RawFd"]),
            (Blessing::BorrowedFd, &["AsRawFd", "BorrowedFd", "RawFd"]),
            (Blessing::OwnedFd, &["FromRawFd", "OwnedFd", "RawFd"]),
        ];
        let mut symbols: Vec<&str> = Vec::new();
        for (blessing, syms) in fd_blessing_symbols {
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
            writeln!(out, "#[cfg(unix)]").unwrap();
            writeln!(out, "use std::os::unix::io::{{{list}}};").unwrap();
            writeln!(out).unwrap();
        }

        let handle_blessing_symbols: &[(Blessing, &[&str])] = &[
            (Blessing::RawHandle, &["RawHandle"]),
            (
                Blessing::BorrowedHandle,
                &["AsRawHandle", "BorrowedHandle", "RawHandle"],
            ),
            (
                Blessing::OwnedHandle,
                &["FromRawHandle", "OwnedHandle", "RawHandle"],
            ),
        ];
        let mut symbols: Vec<&str> = Vec::new();
        for (blessing, syms) in handle_blessing_symbols {
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
            writeln!(out, "#[cfg(windows)]").unwrap();
            writeln!(out, "use std::os::windows::io::{{{list}}};").unwrap();
            writeln!(out).unwrap();
        }
    }

    // Emit use statements for foreign handle types from other ffier libraries.
    {
        let mut foreign_imports: HashMap<&str, Vec<&str>> = HashMap::new();
        for (name, entry) in &lib.type_registry {
            if let TypeKind::ForeignHandle { foreign_crate, .. } = &entry.kind {
                foreign_imports
                    .entry(foreign_crate.as_str())
                    .or_default()
                    .push(name.as_str());
            }
        }
        let mut crates: Vec<_> = foreign_imports.keys().copied().collect();
        crates.sort();
        for crate_path in crates {
            let mut types = foreign_imports[crate_path].clone();
            types.sort();
            let list = types.join(", ");
            writeln!(out, "use {crate_path}::{{{list}}};").unwrap();
        }
        if !foreign_imports.is_empty() {
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
        "    /// # Safety\n    /// The caller must ensure the handle is not used after the object is dropped."
    )
    .unwrap();
    writeln!(
        out,
        "    unsafe fn as_handle(&self) -> *mut core::ffi::c_void;"
    )
    .unwrap();
    writeln!(
        out,
        "    /// # Safety\n    /// Handle must be valid, uniquely owned, and valid for every lifetime of Self.\n    unsafe fn __from_raw(handle: *mut core::ffi::c_void) -> Self;"
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
    writeln!(
        out,
        "    /// # Safety\n    /// The C representation must be valid for this type."
    )
    .unwrap();
    writeln!(out, "    unsafe fn from_c(repr: Self::CRepr) -> Self;").unwrap();

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Primitive FfiType impls
    writeln!(out, "macro_rules! impl_ffi_identity {{").unwrap();
    writeln!(out, "    ($($t:ty => $n:expr),* $(,)?) => {{ $(").unwrap();
    writeln!(out, "        impl FfiType for $t {{").unwrap();
    writeln!(out, "            type CRepr = $t; const C_TYPE_NAME: &'static str = $n; const IS_HANDLE: bool = false;").unwrap();
    writeln!(
        out,
        "            fn into_c(self) -> Self {{ self }} unsafe fn from_c(r: Self) -> Self {{ r }}"
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
        "    isize => \"ssize_t\", usize => \"size_t\", bool => \"bool\", f32 => \"float\", f64 => \"double\","
    )
    .unwrap();
    writeln!(
        out,
        "    *mut core::ffi::c_void => \"void*\", *const core::ffi::c_void => \"const void*\","
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // &str FfiType impl
    writeln!(out, "impl FfiType for &str {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierStr\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ unsafe {{ ffier::FfierBytes::from_str(self) }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ unsafe {{ let b = core::slice::from_raw_parts(repr.data, repr.len); core::str::from_utf8_unchecked(b) }} }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Option<&str> FfiType impl
    writeln!(out, "impl FfiType for Option<&str> {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierStr\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ match self {{ Some(s) => unsafe {{ ffier::FfierBytes::from_str(s) }}, None => ffier::FfierBytes::EMPTY }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ if repr.data.is_null() {{ None }} else {{ unsafe {{ Some(core::str::from_utf8_unchecked(core::slice::from_raw_parts(repr.data, repr.len))) }} }} }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Box<str> FfiType impl
    writeln!(out, "impl FfiType for Box<str> {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierStr\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ let leaked: &mut str = Box::leak(self); ffier::FfierBytes {{ data: leaked.as_mut_ptr() as *const u8, len: leaked.len() }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ unsafe {{ let slice = core::slice::from_raw_parts_mut(repr.data as *mut u8, repr.len); Box::from_raw(core::str::from_utf8_unchecked_mut(slice)) }} }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // &[u8] FfiType impl
    writeln!(out, "impl FfiType for &[u8] {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierBytes\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ unsafe {{ ffier::FfierBytes::from_bytes(self) }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ unsafe {{ if repr.data.is_null() {{ &[] }} else {{ core::slice::from_raw_parts(repr.data, repr.len) }} }} }}").unwrap();

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Mutable byte buffers use a writable pointer and preserve exclusivity.
    writeln!(out, "impl FfiType for &mut [u8] {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierMutBytes; const C_TYPE_NAME: &'static str = \"FfierBytesMut\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierMutBytes {{ unsafe {{ ffier::FfierMutBytes::from_bytes(self) }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierMutBytes) -> Self {{ unsafe {{ repr.into_bytes() }} }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // OS paths reuse the byte-slice repr but keep OsStr's internal
    // representation rather than going through a platform string type.
    writeln!(out, "impl FfiType for &Path {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierPath\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ unsafe {{ ffier::FfierBytes::from_path(self) }} }}").unwrap();
    writeln!(
        out,
        "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ unsafe {{ repr.as_path() }} }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "impl FfiType for Option<&Path> {{").unwrap();
    writeln!(out, "    type CRepr = ffier::FfierBytes; const C_TYPE_NAME: &'static str = \"FfierPath\"; const IS_HANDLE: bool = false;").unwrap();
    writeln!(out, "    fn into_c(self) -> ffier::FfierBytes {{ match self {{ Some(p) => unsafe {{ ffier::FfierBytes::from_path(p) }}, None => ffier::FfierBytes::EMPTY }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(repr: ffier::FfierBytes) -> Self {{ if repr.data.is_null() {{ None }} else {{ unsafe {{ Some(repr.as_path()) }} }} }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Blessed fd type FfiType impls
    emit_blessed_fd_impls(&mut out, lib);

    // Blanket FfiType for &T / &mut T — resolves CRepr to *mut c_void for
    // extern declarations. from_c is unused (bridge calls ffier_handle_borrow
    // directly). into_c is unused (wrapper calls as_handle directly).
    writeln!(out, "impl<T: FfiHandle + 'static> FfiType for &T {{").unwrap();
    writeln!(out, "    type CRepr = *mut core::ffi::c_void; const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME; const IS_HANDLE: bool = true;").unwrap();
    writeln!(out, "    fn into_c(self) -> *mut core::ffi::c_void {{ unsafe {{ FfiHandle::as_handle(self) }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(_: *mut core::ffi::c_void) -> Self {{ unimplemented!(\"&T from_c\") }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "impl<T: FfiHandle + 'static> FfiType for &mut T {{").unwrap();
    writeln!(out, "    type CRepr = *mut core::ffi::c_void; const C_TYPE_NAME: &'static str = T::C_HANDLE_NAME; const IS_HANDLE: bool = true;").unwrap();
    writeln!(out, "    fn into_c(self) -> *mut core::ffi::c_void {{ unsafe {{ FfiHandle::as_handle(self) }} }}").unwrap();
    writeln!(out, "    unsafe fn from_c(_: *mut core::ffi::c_void) -> Self {{ unimplemented!(\"&mut T from_c\") }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiHandle + FfiType impls for foreign handle types — delegate to the
    // foreign crate's traits so foreign types are used identically to local
    // types throughout the generated code.
    for (name, entry) in &lib.type_registry {
        if let TypeKind::ForeignHandle { foreign_crate, .. } = &entry.kind {
            let fc = foreign_crate;
            let lifetime_args = entry
                .lifetime_params
                .iter()
                .map(|lt| format!("'{}", lt.name))
                .collect::<Vec<_>>()
                .join(", ");
            let impl_generics = if lifetime_args.is_empty() {
                String::new()
            } else {
                format!("<{lifetime_args}>")
            };
            let type_name = if lifetime_args.is_empty() {
                name.clone()
            } else {
                format!("{name}<{lifetime_args}>")
            };
            writeln!(out, "impl{impl_generics} FfiHandle for {type_name} {{").unwrap();
            writeln!(out, "    const C_HANDLE_NAME: &'static str = <{type_name} as {fc}::FfiHandle>::C_HANDLE_NAME;").unwrap();
            writeln!(
                out,
                "    const TYPE_TAG: u32 = <{type_name} as {fc}::FfiHandle>::TYPE_TAG;"
            )
            .unwrap();
            writeln!(out, "    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {{ unsafe {{ {fc}::FfiHandle::as_handle(self) }} }}").unwrap();
            writeln!(out, "    unsafe fn __from_raw(handle: *mut core::ffi::c_void) -> Self {{ unsafe {{ <{type_name} as {fc}::FfiHandle>::__from_raw(handle) }} }}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out, "impl{impl_generics} FfiType for {type_name} {{").unwrap();
            writeln!(out, "    type CRepr = *mut core::ffi::c_void;").unwrap();
            writeln!(
                out,
                "    const C_TYPE_NAME: &'static str = <{type_name} as {fc}::FfiHandle>::C_HANDLE_NAME;"
            )
            .unwrap();
            writeln!(
                out,
                "    fn into_c(self) -> *mut core::ffi::c_void {{ {fc}::FfiType::into_c(self) }}"
            )
            .unwrap();
            writeln!(out, "    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {{ unsafe {{ <{type_name} as {fc}::FfiHandle>::__from_raw(repr) }} }}").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
        }
    }

    // ForeignSlice — typed wrapper around FfierObjectArray. Only a library
    // that returns handle slices needs it, so it is pruned along with its
    // free symbol.
    if object_array_returns {
        writeln!(
            out,
            "/// Borrowed slice of handles returned from FFI methods."
        )
        .unwrap();
        writeln!(
            out,
            "/// Elements are borrowed — do NOT destroy them individually."
        )
        .unwrap();
        writeln!(
            out,
            "/// Derefs to `&[T]`, so indexing, iteration, `len()`, etc. all work."
        )
        .unwrap();
        writeln!(out, "/// Dropping this type frees the backing array.").unwrap();
        writeln!(out, "pub struct ForeignSlice<T> {{").unwrap();
        writeln!(out, "    raw: ffier::FfierObjectArray,").unwrap();
        writeln!(out, "    elements: Box<[core::mem::ManuallyDrop<T>]>,").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "impl<T: FfiHandle> ForeignSlice<T> {{").unwrap();
        writeln!(
            out,
            "    fn from_raw(raw: ffier::FfierObjectArray) -> Self {{"
        )
        .unwrap();
        writeln!(
            out,
            "        let elements: Box<[core::mem::ManuallyDrop<T>]> = (0..raw.len)"
        )
        .unwrap();
        writeln!(
            out,
            "            .map(|i| core::mem::ManuallyDrop::new(unsafe {{ T::__from_raw("
        )
        .unwrap();
        writeln!(
            out,
            "                ffier::ffier_object_array_get(raw, i)) }}))"
        )
        .unwrap();
        writeln!(out, "            .collect();").unwrap();
        writeln!(out, "        Self {{ raw, elements }}").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "impl<T> core::ops::Deref for ForeignSlice<T> {{").unwrap();
        writeln!(out, "    type Target = [T];").unwrap();
        writeln!(out, "    fn deref(&self) -> &[T] {{").unwrap();
        writeln!(
            out,
            "        // SAFETY: ManuallyDrop<T> is #[repr(transparent)] over T."
        )
        .unwrap();
        writeln!(out, "        unsafe {{ core::mem::transmute::<&[core::mem::ManuallyDrop<T>], &[T]>(&self.elements) }}").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "impl<T> Drop for ForeignSlice<T> {{").unwrap();
        writeln!(out, "    fn drop(&mut self) {{").unwrap();
        // The array was allocated inside the library, so free it there. The
        // runtime helper would reconstruct the allocation with this crate's
        // allocator, which need not match.
        writeln!(
            out,
            "        unsafe {{ {}_free_object_array(self.raw) }};",
            lib.prefix
        )
        .unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    let mut symbols = Vec::new();

    // 0. Enum constants (FfiType impls + constant values)
    for en in &lib.enum_constants {
        emit_enum_type(&mut out, en, lib);
    }

    // 0b. Bitflags constants (bitflags! invocations + FfiType impls)
    for bf in &lib.bitflags_constants {
        emit_bitflags_type(&mut out, bf, lib);
    }

    // 0c. Fixed-layout value structs
    emit_value_structs(&mut out, lib);

    // 1. Error enums
    // Emit shared error factory, payload, and static message declarations once.
    if !lib.errors.is_empty() {
        let payload_fn = format!("{}_error_payload", lib.prefix);
        let mut error_fns = vec![
            extern_fn_private(
                &format!("{}_error_new", lib.prefix),
                vec!["r: ffier::FfierResult".to_string()],
                " -> *mut core::ffi::c_void".to_string(),
            ),
            extern_fn_private(
                &payload_fn,
                vec![
                    "handle: *const core::ffi::c_void".to_string(),
                    "index: usize".to_string(),
                    "out_buf: *mut core::ffi::c_void".to_string(),
                    "buf_size: usize".to_string(),
                ],
                String::new(),
            ),
        ];
        if lib
            .errors
            .iter()
            .flat_map(|error| &error.variants)
            .any(|variant| variant.data.fields().is_empty())
        {
            error_fns.push(extern_fn_private(
                &format!("{}_result_message", lib.prefix),
                vec!["r: ffier::FfierResult".to_string()],
                " -> ffier::FfierBytes".to_string(),
            ));
        }
        emit_extern_fns(&mut out, &error_fns, None, &mut symbols);
    }
    for err in &lib.errors {
        emit_error(&mut out, err, lib);
    }

    // 2. Exported types
    for ty in &lib.exported_types {
        emit_exported_type(&mut out, ty, lib, &mut symbols);
    }

    // Library-side allocation helpers. Memory the library allocates must be
    // freed by the library, so both of these are library symbols rather than
    // ffier:: runtime helpers.
    let mut alloc_fns = Vec::new();
    if let Some(name) = &handle_allocator {
        alloc_fns.push(extern_fn_private(
            name,
            vec![
                "type_tag: u32".to_string(),
                "vtable: *const core::ffi::c_void".to_string(),
                "user_data: *const core::ffi::c_void".to_string(),
            ],
            " -> *mut core::ffi::c_void".to_string(),
        ));
    }
    if object_array_returns {
        alloc_fns.push(extern_fn_private(
            &format!("{}_free_object_array", lib.prefix),
            vec!["a: ffier::FfierObjectArray".to_string()],
            String::new(),
        ));
    }
    if !alloc_fns.is_empty() {
        emit_extern_fns(&mut out, &alloc_fns, None, &mut symbols);
    }

    // 3. Implementable traits
    // Build trait defaults map: trait_name → vec of defaulted methods
    let mut trait_defaults: HashMap<String, Vec<&Method>> = HashMap::new();
    for tr in &lib.traits {
        let defaults: Vec<_> = tr
            .methods
            .iter()
            .filter(|m| m.trait_definition.as_ref().is_some_and(|td| td.has_default))
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
            emit_error_trait_externs(&mut out, tr, lib, &mut symbols);
            defined_traits.insert(tr.name.clone());
            continue;
        }
        // no_vtable traits have no vtable/wrapper — let emit_trait_impl
        // define the trait via emit_simple_trait_def when concrete impls
        // are encountered.
        if tr.no_vtable {
            continue;
        }
        emit_implementable_trait(&mut out, tr, lib, &mut symbols);
        defined_traits.insert(tr.name.clone());
    }

    // 4. Trait impls
    for ti in &lib.trait_impls {
        let is_handle = lib
            .type_entry(&ti.struct_name)
            .is_some_and(|e| e.kind.is_handle());
        if !is_handle {
            continue;
        }
        emit_trait_impl(
            &mut out,
            ti,
            lib,
            &mut defined_traits,
            &trait_defaults,
            &mut symbols,
        );
    }

    // 5. Free functions
    for f in &lib.free_functions {
        // Free functions already handle their own cfg in emit_free_function;
        // type-level cfg is propagated to the schema's FreeFunction.cfg field.
        emit_free_function(&mut out, f, lib, &mut symbols);
    }

    // 6. Namespaced loading/configuration API and symbol caches.
    if !symbols.is_empty() {
        emit_symbol_api(&mut out, &symbols, handle_allocator.as_deref());
    }

    rustfmt(&out)
}
