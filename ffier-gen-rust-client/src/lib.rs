//! Rust client bindings generator from ffier JSON schema.
//!
//! Reads a `ffier-{prefix}.json` and produces a complete Rust source file with:
//! - Error enums with `from_ffi`, `Display`, `Error` impls
//! - Handle wrapper structs with extern "C" declarations
//! - Safe method wrappers with lifetime-correct signatures
//! - Trait definitions with vtable structs for exported traits
//! - Trait impl blocks for concrete types
//!
//! The generated source supports Unix runtime lookup (`dynlookup`, the consumer's
//! default feature) and linker-resolved calls (`strong`, which always wins).
//! The consuming crate must declare these features and make `libloading = "0.8"`
//! optional through `dynlookup = ["dep:libloading"]`. Strong-only consumers can disable
//! default features to avoid that dependency. Intermediate libraries can leave
//! mode selection to the final application through Cargo feature unification.
//! Windows loading support is outside this interface refactor.

use ffier_schema::{
    EnumType, ErrorType, ErrorVariant, ErrorVariantData, ExportedType, FreeFunction,
    ImplementableTrait, Library, Method, ParamType, Receiver, Return, TraitImpl, TypeKind,
};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// Emit schema restrictions only for strong linking; dynlookup clients discover
/// optional API at runtime regardless of the consumer's schema features.
fn emit_cfg_attr(out: &mut String, cfg: Option<&str>) {
    if let Some(pred) = cfg {
        writeln!(out, "#[cfg(any(not(feature = \"strong\"), {pred}))]").unwrap();
    }
}

/// Resolve dependencies before any argument or receiver loses Rust ownership.
fn emit_symbol_preflight(out: &mut String, name: &str, indent: &str) {
    let variant = symbol_to_variant(name);
    let upper = name.to_ascii_uppercase();
    writeln!(out, "{indent}#[cfg(not(feature = \"strong\"))]").unwrap();
    writeln!(out, "{indent}__ffier_symbol(Symbol::{variant}, &{upper});").unwrap();
}

fn emit_call_preflight(out: &mut String, name: &str, params: &[ffier_schema::Param], indent: &str) {
    emit_symbol_preflight(out, name, indent);
    // Callback implementations prepare their allocation symbol; concrete
    // handles override this hook because transferring them does not allocate.
    for param in params {
        if matches!(param.param_type, ParamType::ImplTrait { .. }) {
            writeln!(out, "{indent}{}.__ffier_prepare();", param.name).unwrap();
        }
    }
}

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

/// Generate one Rust client source supporting Unix dynlookup and strong linking.
/// The consuming crate selects the implementation with its `dynlookup` (default)
/// and `strong` features. `strong` takes precedence when both are enabled.
pub fn generate(lib: &Library) -> String {
    let mut out = String::new();
    out.push_str("#[cfg(not(any(feature = \"dynlookup\", feature = \"strong\")))]\ncompile_error!(\"generated ffier clients require the `dynlookup` or `strong` feature\");\n\n");
    let object_array_returns = has_object_array_returns(lib);

    // Emit imports for blessed types that need special std imports.
    // Each blessing contributes individual import symbols; collected and
    // emitted as a single `use` statement to avoid duplicates.
    {
        use ffier_schema::Blessing;
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
        "    fn __from_raw(handle: *mut core::ffi::c_void) -> Self;"
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
                .map(|lt| format!("'{lt}"))
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
            writeln!(out, "    fn __from_raw(handle: *mut core::ffi::c_void) -> Self {{ <{type_name} as {fc}::FfiHandle>::__from_raw(handle) }}").unwrap();
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
            writeln!(out, "    unsafe fn from_c(repr: *mut core::ffi::c_void) -> Self {{ <{type_name} as {fc}::FfiHandle>::__from_raw(repr) }}").unwrap();
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
            "            .map(|i| core::mem::ManuallyDrop::new(T::__from_raw("
        )
        .unwrap();
        writeln!(
            out,
            "                unsafe {{ ffier::ffier_object_array_get(raw, i) }})))"
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
    if lib.traits.iter().any(|tr| !tr.no_vtable) {
        alloc_fns.push(extern_fn_private(
            &format!("{}_vtable_handle_new", lib.prefix),
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

    // 6. Shared symbol API (Symbol enum + require + is_available)
    if !symbols.is_empty() {
        emit_symbol_api(&mut out, &symbols);
    }

    rustfmt(&out)
}

/// Run `rustfmt` on the generated source. Panics if `rustfmt` is not
/// available or if it fails (the generated code has a syntax error).
fn rustfmt(src: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn rustfmt — is it installed?");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(src.as_bytes())
        .expect("failed to write to rustfmt stdin");

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "rustfmt failed on generated code:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("rustfmt produced non-UTF-8 output")
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

fn fieldless_variant_syntax(variant: &ErrorVariant) -> Option<String> {
    match &variant.data {
        ErrorVariantData::Fieldless => Some(variant.name.clone()),
        ErrorVariantData::Tuple(fields) if fields.is_empty() => Some(format!("{}()", variant.name)),
        ErrorVariantData::Tuple(_) => None,
    }
}

fn emit_error(out: &mut String, err: &ErrorType, lib: &Library) {
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
        emit_symbol_preflight(out, error_destroy_fn, "        ");
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
        .filter(|e| e.kind.is_handle())
        .map(|_| tr.type_name.as_str())
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

fn emit_exported_type(
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
                .map(|lt| format!("'{lt}"))
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
    emit_extern_fns(out, &fns, ty.cfg.as_deref(), symbols);

    // Struct definition
    emit_cfg_attr(out, ty.cfg.as_deref());
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
    emit_cfg_attr(out, ty.cfg.as_deref());
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
    emit_cfg_attr(out, ty.cfg.as_deref());
    writeln!(
        out,
        "impl{lt_params} FfiHandle for {}{lt_params} {{",
        ty.name
    )
    .unwrap();
    let c_handle_name = lib.c_type_of(&ty.name);
    writeln!(
        out,
        "    const C_HANDLE_NAME: &'static str = \"{c_handle_name}\";",
    )
    .unwrap();
    writeln!(out, "    const TYPE_TAG: u32 = {type_tag}u32;").unwrap();
    writeln!(
        out,
        "    unsafe fn as_handle(&self) -> *mut core::ffi::c_void {{ self.0 }}"
    )
    .unwrap();
    if has_lifetimes {
        writeln!(out, "    fn __from_raw(handle: *mut core::ffi::c_void) -> Self {{ Self(handle, std::marker::PhantomData) }}").unwrap();
    } else {
        writeln!(
            out,
            "    fn __from_raw(handle: *mut core::ffi::c_void) -> Self {{ Self(handle) }}"
        )
        .unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiType
    emit_cfg_attr(out, ty.cfg.as_deref());
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
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Debug
    emit_cfg_attr(out, ty.cfg.as_deref());
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
    emit_cfg_attr(out, ty.cfg.as_deref());
    writeln!(out, "impl{lt_params} {}{lt_params} {{", ty.name).unwrap();
    for m in &ty.methods {
        emit_method_wrapper(out, m, &m.ffi_name, ty.is_builder_type, has_lifetimes, lib);
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
        emit_cfg_attr(out, ty.cfg.as_deref());
        emit_cfg_attr(out, new.cfg.as_deref());
        writeln!(
            out,
            "impl Default for {} {{ fn default() -> Self {{ Self::new() }} }}",
            ty.name
        )
        .unwrap();
        writeln!(out).unwrap();
    }

    // Drop
    emit_cfg_attr(out, ty.cfg.as_deref());
    writeln!(out, "impl{lt_params} Drop for {}{lt_params} {{", ty.name).unwrap();
    writeln!(out, "    fn drop(&mut self) {{").unwrap();
    writeln!(out, "        unsafe {{ {}(self.0) }}", ty.destroy_ffi_name).unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
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
    emit_cfg_attr(out, m.cfg.as_deref());

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

    let param_sigs: Vec<String> = m.params.iter().map(format_param_sig).collect();
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
        Return::ObjectArray { element } => {
            format!(" -> ForeignSlice<{}>", element.type_name)
        }
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
    emit_call_preflight(out, ffi_name, &m.params, "        ");
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
        Return::ObjectArray { .. } => {
            // Object array: wrap raw FfierObjectArray in ForeignSlice.
            writeln!(
                out,
                "        ForeignSlice::from_raw(unsafe {{ {ffi_name}({args_str}) }})"
            )
            .unwrap();
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
                writeln!(out, "        <{bare_name} as FfiHandle>::__from_raw(__raw)").unwrap();
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
                    "            Ok(<{bare_name} as FfiHandle>::__from_raw(__raw))"
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

/// Emit only the extern declarations for the error trait's dispatch functions.
/// This avoids emitting a `pub trait Error { ... }` that would collide with
/// the user's error enum, while still making the symbols available for
/// GLib-style Result wrappers.
fn emit_error_trait_externs(
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
    emit_extern_fns(out, &fns, tr.cfg.as_deref(), symbols);
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
        .map(|m| m.ffi_name.as_str())
        .expect("error trait has no 'result' method");
    (result_fn, &error_trait.destroy_ffi_name)
}

// ===========================================================================
// Implementable trait generation
// ===========================================================================

fn emit_implementable_trait(
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
    emit_cfg_attr(out, tr.cfg.as_deref());
    writeln!(out, "pub trait {} {{", tr.name).unwrap();
    for m in &tr.methods {
        let Some(td) = &m.trait_definition else {
            continue;
        };

        emit_cfg_attr(out, m.cfg.as_deref());
        let method_sig = build_trait_method_sig(m, lib);

        if td.has_default {
            // Default impl via self-dispatch
            writeln!(out, "    {method_sig} where Self: Sized {{").unwrap();
            emit_default_dispatch_body(out, m, lib, type_tag, vtable_name, td.index);
            writeln!(out, "    }}").unwrap();
        } else {
            writeln!(out, "    {method_sig};").unwrap();
        }
    }

    // Preparation borrows the value, so failed lookup leaves ownership intact.
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(out, "    fn __ffier_prepare(&self) {{").unwrap();
    emit_symbol_preflight(
        out,
        &format!("{}_vtable_handle_new", lib.prefix),
        "        ",
    );
    writeln!(out, "    }}").unwrap();

    // __ffier_vtable
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(
        out,
        "    fn __ffier_vtable() -> &'static {vtable_name} where Self: Sized {{"
    )
    .unwrap();
    emit_vtable_constructor(out, tr);
    writeln!(out, "    }}").unwrap();

    // __into_raw_handle
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(
        out,
        "    fn __into_raw_handle(self) -> *mut core::ffi::c_void where Self: Sized {{"
    )
    .unwrap();
    writeln!(out, "        self.__ffier_prepare();").unwrap();
    writeln!(
        out,
        "        let __vtable: &'static {vtable_name} = Self::__ffier_vtable();"
    )
    .unwrap();
    // The consuming callee frees this handle, so the library must allocate it.
    // The vtable drop trampoline releases user_data on the client side.
    writeln!(
        out,
        "        let __user_data = Box::into_raw(Box::new(self));"
    )
    .unwrap();
    writeln!(out, "        unsafe {{").unwrap();
    writeln!(out, "            {}_vtable_handle_new(", lib.prefix).unwrap();
    writeln!(out, "                {type_tag}u32,").unwrap();
    writeln!(
        out,
        "                __vtable as *const {vtable_name} as *const core::ffi::c_void,"
    )
    .unwrap();
    writeln!(
        out,
        "                __user_data as *const core::ffi::c_void,"
    )
    .unwrap();
    writeln!(out, "            )").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Vtable struct
    emit_vtable_struct(out, tr, lib, vtable_name);

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
        emit_extern_fns(out, &default_fns, tr.cfg.as_deref(), symbols);
    }

    // Wrapper struct
    emit_cfg_attr(out, tr.cfg.as_deref());
    writeln!(out, "pub struct {wrapper_name}(*mut core::ffi::c_void);").unwrap();
    writeln!(out).unwrap();
    emit_cfg_attr(out, tr.cfg.as_deref());
    writeln!(out, "impl {wrapper_name} {{").unwrap();
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(out, "    pub fn __into_raw(self) -> *mut core::ffi::c_void {{ let this = std::mem::ManuallyDrop::new(self); this.0 }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    emit_cfg_attr(out, tr.cfg.as_deref());
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

    let params: Vec<String> = m.params.iter().map(format_param_sig).collect();
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
    let dispatch_fn = &m.ffi_name;
    emit_call_preflight(out, dispatch_fn, &m.params, "        ");

    writeln!(
        out,
        "        let __vtable: &'static {vtable_name} = Self::__ffier_vtable();"
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

    emit_slice_pre_bindings(out, &m.params, "        ");

    let mut call_args = vec![
        "&mut __temp as *mut ffier::FfierHandle<ffier::VtableHandle> as *mut core::ffi::c_void"
            .to_string(),
    ];
    call_args.extend(build_ffi_param_args(&m.params, lib));
    let args_str = call_args.join(", ");

    emit_ffi_call_return(out, dispatch_fn, &args_str, &m.ret, "        ", lib);
}

fn emit_vtable_struct(
    out: &mut String,
    tr: &ImplementableTrait,
    _lib: &Library,
    vtable_name: &str,
) {
    emit_cfg_attr(out, tr.cfg.as_deref());
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

    let mut method_by_index: HashMap<usize, &Method> = HashMap::new();
    for m in &tr.methods {
        if let Some(td) = &m.trait_definition {
            method_by_index.insert(td.index, m);
        }
    }

    for slot in 0..tr.num_slots {
        if let Some(m) = method_by_index.get(&slot) {
            emit_cfg_attr(out, m.cfg.as_deref());
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
            if let Some(cfg) = &m.cfg {
                writeln!(out, "#[cfg(all(feature = \"strong\", not({cfg})))]").unwrap();
                writeln!(
                    out,
                    "    pub __reserved_{slot}: Option<unsafe extern \"C\" fn()>,"
                )
                .unwrap();
            }
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

fn emit_vtable_constructor(out: &mut String, tr: &ImplementableTrait) {
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
    writeln!(out, "                __drop_trampoline::<Self>").unwrap();
    writeln!(out, "            }}),").unwrap();

    let mut method_by_index: HashMap<usize, &Method> = HashMap::new();
    for m in &tr.methods {
        if let Some(td) = &m.trait_definition {
            method_by_index.insert(td.index, m);
        }
    }

    for slot in 0..tr.num_slots {
        if let Some(m) = method_by_index.get(&slot) {
            emit_cfg_attr(out, m.cfg.as_deref());
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
                            writeln!(
                                out,
                                "                            let (_, __handle) = __e.into_ffi();"
                            )
                            .unwrap();
                            writeln!(
                                out,
                                "                            unsafe {{ *err_out = __handle }};"
                            )
                            .unwrap();
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
                            writeln!(out, "                            let (__code, __handle) = __e.into_ffi();").unwrap();
                            writeln!(
                                out,
                                "                            unsafe {{ *err_out = __handle }};"
                            )
                            .unwrap();
                            writeln!(out, "                            __code").unwrap();
                            writeln!(out, "                        }}").unwrap();
                        }
                    }
                    writeln!(out, "                    }}").unwrap();
                }
            }

            writeln!(out, "                }}").unwrap();
            writeln!(out, "                __trampoline::<Self>").unwrap();
            writeln!(out, "            }}),").unwrap();
            if let Some(cfg) = &m.cfg {
                writeln!(out, "#[cfg(all(feature = \"strong\", not({cfg})))]").unwrap();
                writeln!(out, "            __reserved_{slot}: None,").unwrap();
            }
        } else {
            writeln!(out, "            __reserved_{slot}: None,").unwrap();
        }
    }

    writeln!(out, "        }}").unwrap();
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
    emit_extern_fns(out, &fns, ti.cfg.as_deref(), symbols);

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

    emit_cfg_attr(out, ti.cfg.as_deref());
    writeln!(
        out,
        "impl{impl_generics} {}{trait_args} for {}{struct_args} {{",
        ti.trait_name, ti.struct_name
    )
    .unwrap();

    // Emit methods from this impl
    let impl_method_names: HashSet<&str> = ti.methods.iter().map(|m| m.name.as_str()).collect();

    for m in &ti.methods {
        emit_cfg_attr(out, m.cfg.as_deref());
        let sig = build_trait_method_sig(m, lib);
        writeln!(out, "    {sig} {{").unwrap();
        emit_call_preflight(out, &m.ffi_name, &m.params, "        ");

        emit_slice_pre_bindings(out, &m.params, "        ");
        let mut ffi_args = vec!["self.0".to_string()];
        ffi_args.extend(build_ffi_param_args(&m.params, lib));
        let args_str = ffi_args.join(", ");

        emit_ffi_call_return(out, &m.ffi_name, &args_str, &m.ret, "        ", lib);
        writeln!(out, "    }}").unwrap();
    }

    // Emit default method forwarders (from trait defaults not in this impl)
    if let Some(defaults) = trait_defaults.get(&ti.trait_name) {
        for dm in defaults {
            if impl_method_names.contains(dm.name.as_str()) {
                continue;
            }
            emit_cfg_attr(out, dm.cfg.as_deref());
            let sig = build_trait_method_sig(dm, lib);
            writeln!(out, "    {sig} {{").unwrap();
            emit_call_preflight(out, &dm.ffi_name, &dm.params, "        ");

            emit_slice_pre_bindings(out, &dm.params, "        ");
            let mut ffi_args = vec!["self.0".to_string()];
            ffi_args.extend(build_ffi_param_args(&dm.params, lib));
            let args_str = ffi_args.join(", ");

            emit_ffi_call_return(out, &dm.ffi_name, &args_str, &dm.ret, "        ", lib);
            writeln!(out, "    }}").unwrap();
        }
    }

    // Concrete handle transfers do not need callback allocation.
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(out, "    fn __ffier_prepare(&self) {{}}").unwrap();

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

    let cfg = lib
        .traits
        .iter()
        .find(|tr| tr.name == ti.trait_name)
        .and_then(|tr| tr.cfg.as_deref());
    emit_cfg_attr(out, cfg);
    writeln!(out, "pub trait {}{trait_generics} {{", ti.trait_name).unwrap();
    for m in &ti.methods {
        emit_cfg_attr(out, m.cfg.as_deref());
        let sig = build_trait_method_sig(m, lib);
        writeln!(out, "    {sig};").unwrap();
    }
    writeln!(out, "    #[doc(hidden)]").unwrap();
    writeln!(out, "    fn __ffier_prepare(&self) {{}}").unwrap();
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
// Extern / dynlookup-shim emission
// ===========================================================================

/// A single extern "C" function declaration to emit.
struct ExternFn {
    /// Function name (e.g. `"ft_widget_new"`).
    name: String,
    /// Typed parameters (e.g. `["handle: *mut core::ffi::c_void", "x: i32"]`).
    params: Vec<String>,
    /// Return type string including ` -> ...`, or empty for void.
    ret: String,
    /// Whether to emit as `pub fn` (true) or `fn` (false) in extern blocks.
    public: bool,
    /// `#[cfg(...)]` predicate string. `None` = unconditional.
    cfg: Option<String>,
}

/// Build a `pub` `ExternFn` from a name, param list, and return string.
fn extern_fn(name: &str, params: Vec<String>, ret: String) -> ExternFn {
    ExternFn {
        name: name.to_string(),
        params,
        ret,
        public: true,
        cfg: None,
    }
}

/// Build a `pub` `ExternFn` with an optional cfg predicate.
fn extern_fn_cfg(name: &str, params: Vec<String>, ret: String, cfg: Option<String>) -> ExternFn {
    ExternFn {
        name: name.to_string(),
        params,
        ret,
        public: true,
        cfg,
    }
}

/// Build a non-`pub` `ExternFn`.
fn extern_fn_private(name: &str, params: Vec<String>, ret: String) -> ExternFn {
    ExternFn {
        name: name.to_string(),
        params,
        ret,
        public: false,
        cfg: None,
    }
}

/// Build an `ExternFn` from a method's signature.
fn extern_fn_from_method(ffi_name: &str, m: &Method, lib: &Library) -> ExternFn {
    let is_builder = m.ret.is_builder_self(&lib.type_registry);
    let mut params = Vec::new();
    match m.receiver {
        Receiver::None => {}
        _ => {
            params.push("handle: *mut core::ffi::c_void".to_string());
        }
    }
    push_extern_params(&m.params, &mut params);
    let ret = push_return_and_out_params(&m.ret, &mut params, is_builder);
    extern_fn_cfg(ffi_name, params, ret, m.cfg.clone())
}

/// Build an `ExternFn` from a dispatch method signature.
fn extern_fn_from_dispatch(ffi_name: &str, m: &Method) -> ExternFn {
    let mut params = vec!["handle: *mut core::ffi::c_void".to_string()];
    push_extern_params(&m.params, &mut params);
    let ret = push_return_and_out_params(&m.ret, &mut params, false);
    extern_fn_cfg(ffi_name, params, ret, m.cfg.clone())
}

/// Build an `ExternFn` from a free function.
fn extern_fn_from_free(f: &FreeFunction) -> ExternFn {
    let mut params = Vec::new();
    push_extern_params(&f.params, &mut params);
    let ret = push_return_and_out_params(&f.ret, &mut params, false);
    extern_fn_cfg(&f.ffi_name, params, ret, f.cfg.clone())
}

/// Emit complementary strong declarations and dynlookup shims with shared names.
fn emit_extern_fns(
    out: &mut String,
    fns: &[ExternFn],
    cfg: Option<&str>,
    symbols: &mut Vec<SymbolInfo>,
) {
    if fns.is_empty() {
        return;
    }
    writeln!(out, "#[cfg(feature = \"strong\")]").unwrap();
    if let Some(pred) = cfg {
        writeln!(out, "#[cfg({pred})]").unwrap();
    }
    writeln!(out, "unsafe extern \"C\" {{").unwrap();
    for f in fns {
        let vis = if f.public { "pub " } else { "" };
        let params_str = f.params.join(", ");
        if let Some(cfg) = &f.cfg {
            writeln!(out, "    #[cfg({cfg})]").unwrap();
        }
        writeln!(out, "    {vis}fn {}({params_str}){};", f.name, f.ret).unwrap();
    }
    writeln!(out, "}}\n").unwrap();
    for f in fns {
        emit_dynlookup_shim(out, f, symbols);
    }
}

/// Convert a C symbol name like `ft_widget_new` to a PascalCase enum variant
/// like `FtWidgetNew`.
fn symbol_to_variant(c_name: &str) -> String {
    c_name
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let mut s = c.to_uppercase().to_string();
                    s.extend(chars);
                    s
                }
            }
        })
        .collect()
}

/// Emit shared metadata and the feature-selected loading API.
fn emit_symbol_api(out: &mut String, symbols: &[SymbolInfo]) {
    out.push_str(
        r#"
/// An FFI symbol, with the same identity in both linking modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::enum_variant_names)] // Preserve the C library prefix in symbol identities.
pub enum Symbol {
"#,
    );
    for s in symbols {
        let variant = symbol_to_variant(&s.c_name);
        writeln!(out, "    /// `{}`\n    {variant},", s.c_name).unwrap();
    }
    out.push_str(
        r#"}
impl Symbol {
    /// The C symbol name.
    pub fn c_name(self) -> &'static str {
        match self {
"#,
    );
    for s in symbols {
        let variant = symbol_to_variant(&s.c_name);
        writeln!(out, "            Symbol::{variant} => \"{}\",", s.c_name).unwrap();
    }
    out.push_str(
        r#"        }
    }

    /// Probe the configured lookup source without initializing the symbol cache.
    /// With `strong`, always returns true: availability reflects the linking
    /// contract, not a runtime probe. Missing symbols are the linker's responsibility.
    pub fn is_available(self) -> bool {
        #[cfg(feature = "strong")]
        {
            true
        }
        #[cfg(not(feature = "strong"))]
        {
            __ffier_with_library(|lib| unsafe {
                lib.get::<*const ()>(self.c_name().as_bytes()).is_ok()
            })
        }
    }

    #[cfg(not(feature = "strong"))]
    fn __load(self, lib: &libloading::os::unix::Library) -> Result<(), LoadError> {
        match self {
"#,
    );
    for s in symbols {
        let variant = symbol_to_variant(&s.c_name);
        let upper = &s.upper;
        let fn_ptr_type = &s.fn_ptr_type;
        writeln!(out, "            Symbol::{variant} => {{").unwrap();
        writeln!(out, "                if {upper}.get().is_none() {{").unwrap();
        writeln!(
            out,
            "                    let f = unsafe {{ *lib.get::<{fn_ptr_type}>(b\"{}\\0\")? }};",
            s.c_name
        )
        .unwrap();
        writeln!(out, "                    let _ = {upper}.set(f);").unwrap();
        writeln!(out, "                }}\n            }}").unwrap();
    }
    out.push_str(
        r#"        }
        Ok(())
    }
}

/// Controls how calls to unloaded symbols are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadPolicy {
    /// Require explicit loading with `require()`; unloaded calls panic.
    Explicit,
    /// Resolve and cache symbols on first use; missing symbols panic.
    Lazy,
}

/// The lookup source and loading policy have already been fixed for this client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetLibraryError;
impl std::fmt::Display for SetLibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("library lookup source is already initialized")
    }
}
impl std::error::Error for SetLibraryError {}

/// Set this generated client's lookup source and unloaded-symbol policy once.
/// `None` selects the process-global namespace. `Some(handle)` borrows an
/// existing `dlopen` handle. All probes, explicit loads, and lazy loads use
/// this same source. Reconfiguration is rejected, even before a symbol is loaded.
/// Without this call, the first probe, load, or unloaded call fixes the source
/// to the process-global namespace with lazy loading disabled.
///
/// `LoadPolicy::Explicit` requires loading with `require()` before calling symbols.
/// `LoadPolicy::Lazy` resolves unloaded symbols on first use and panics if lookup fails.
/// With `strong`, this is a successful no-op and ignores both arguments.
///
/// # Safety
/// An explicit handle must be valid and refer to the library matching these
/// bindings. The caller retains ownership and must keep it loaded for the
/// remaining lifetime of the program. In strong mode the handle is ignored.
pub unsafe fn set_library_handle(
    lib_handle: Option<core::ptr::NonNull<core::ffi::c_void>>,
    load_policy: LoadPolicy,
) -> Result<(), SetLibraryError> {
    #[cfg(feature = "strong")]
    {
        let _ = (lib_handle, load_policy);
        Ok(())
    }
    #[cfg(not(feature = "strong"))]
    {
        let library = lib_handle.map(|handle| std::mem::ManuallyDrop::new(unsafe {
            libloading::os::unix::Library::from_raw(handle.as_ptr())
        }));
        __FFIER_LOOKUP.set(__FfierLookup { library, load_policy }).map_err(|_| SetLibraryError)
    }
}

#[cfg(not(feature = "strong"))]
struct __FfierLookup {
    library: Option<std::mem::ManuallyDrop<libloading::os::unix::Library>>,
    load_policy: LoadPolicy,
}
#[cfg(not(feature = "strong"))]
static __FFIER_LOOKUP: std::sync::OnceLock<__FfierLookup> = std::sync::OnceLock::new();

#[cfg(not(feature = "strong"))]
fn __ffier_lookup() -> &'static __FfierLookup {
    __FFIER_LOOKUP.get_or_init(|| __FfierLookup { library: None, load_policy: LoadPolicy::Explicit })
}

#[cfg(not(feature = "strong"))]
fn __ffier_with_library<R>(operation: impl FnOnce(&libloading::os::unix::Library) -> R) -> R {
    match &__ffier_lookup().library {
        Some(library) => operation(library),
        None => operation(&libloading::os::unix::Library::this()),
    }
}

#[cfg(not(feature = "strong"))]
fn __ffier_symbol<F>(symbol: Symbol, slot: &std::sync::OnceLock<F>) -> &F {
    slot.get().unwrap_or_else(|| {
        match __ffier_lookup().load_policy {
            LoadPolicy::Explicit => panic!(
                "symbol `{}` not loaded; call require() first", symbol.c_name()),
            LoadPolicy::Lazy => {
                __ffier_with_library(|lib| symbol.__load(lib))
                    .unwrap_or_else(|error| panic!("failed to load symbol `{}`: {error}", symbol.c_name()));
                slot.get().expect("loaded symbol cache is empty")
            }
        }
    })
}

/// A symbol loading failure. Strong linking has no loading-failure states.
#[derive(Debug)]
pub enum LoadError {
    /// The original dynamic loader error, without allocation or conversion.
    #[cfg(not(feature = "strong"))]
    Loading(libloading::Error),
}
impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        match *self {
            #[cfg(not(feature = "strong"))]
            Self::Loading(ref error) => std::fmt::Display::fmt(error, f),
        }
    }
}
impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            #[cfg(not(feature = "strong"))]
            Self::Loading(ref error) => Some(error),
        }
    }
}
#[cfg(not(feature = "strong"))]
impl From<libloading::Error> for LoadError {
    fn from(error: libloading::Error) -> Self { Self::Loading(error) }
}

/// Load requested symbols from this client's configured source, skipping
/// initialized caches and returning the first failure. Without a setter call,
/// uses the process-global namespace and disables lazy loading.
/// With `strong`, ignores the symbols and always succeeds.
pub fn require(symbols: &[Symbol]) -> Result<(), LoadError> {
    #[cfg(feature = "strong")]
    {
        let _ = symbols;
        Ok(())
    }
    #[cfg(not(feature = "strong"))]
    {
        __ffier_with_library(|lib| {
            for &symbol in symbols {
                symbol.__load(lib)?;
            }
            Ok(())
        })
    }
}
"#,
    );
}

/// Metadata collected once per FFI symbol for both linking modes.
struct SymbolInfo {
    /// C symbol name (e.g. `"ft_widget_new"`)
    c_name: String,
    /// Rust OnceLock static name (e.g. `"FT_WIDGET_NEW"`)
    upper: String,
    /// fn pointer type string (e.g. `"unsafe extern \"C\" fn() -> *mut c_void"`)
    fn_ptr_type: String,
}

/// Emit a single dynlookup-binding shim: OnceLock + shim fn.
/// Pushes symbol info into `symbols` for the enum/require generation.
fn emit_dynlookup_shim(out: &mut String, f: &ExternFn, symbols: &mut Vec<SymbolInfo>) {
    let name = &f.name;
    let upper = name.to_ascii_uppercase();
    let params_str = f.params.join(", ");
    let ret = &f.ret;

    // Build the fn-pointer type
    let param_types: Vec<&str> = f
        .params
        .iter()
        .map(|p| p.split_once(": ").map(|(_, ty)| ty).unwrap_or(p))
        .collect();
    let fn_ptr_params = param_types.join(", ");
    let fn_ptr_type = format!("unsafe extern \"C\" fn({fn_ptr_params}){ret}");

    // Build the argument-forwarding list (just names)
    let arg_names: Vec<&str> = f
        .params
        .iter()
        .map(|p| p.split_once(": ").map(|(n, _)| n).unwrap_or(p))
        .collect();
    let args_str = arg_names.join(", ");

    // OnceLock
    writeln!(out, "#[cfg(not(feature = \"strong\"))]").unwrap();
    writeln!(
        out,
        "static {upper}: std::sync::OnceLock<{fn_ptr_type}> = std::sync::OnceLock::new();"
    )
    .unwrap();

    // Shim function
    let vis = if f.public { "pub " } else { "" };
    writeln!(out, "#[cfg(not(feature = \"strong\"))]").unwrap();
    writeln!(out, "#[allow(non_snake_case)]").unwrap();
    if f.public {
        writeln!(out, "/// # Safety").unwrap();
        writeln!(out, "/// Uphold this function's C ABI contract, including pointer validity and handle ownership.").unwrap();
    }
    writeln!(out, "{vis}unsafe fn {name}({params_str}){ret} {{").unwrap();
    let variant = symbol_to_variant(name);
    writeln!(
        out,
        "    unsafe {{ (__ffier_symbol(Symbol::{variant}, &{upper}))({args_str}) }}"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();

    symbols.push(SymbolInfo {
        c_name: name.clone(),
        upper: upper.clone(),
        fn_ptr_type,
    });

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
        Return::ObjectArray { .. } => " -> ffier::FfierObjectArray".to_string(),
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
            ParamType::Slice { element, .. } => {
                let elem_ty = element.to_rust_type_static();
                out.push(format!("{}: *const <{elem_ty} as FfiType>::CRepr", p.name));
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
        Return::ObjectArray { .. } => " -> ffier::FfierObjectArray".to_string(),
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
            ParamType::Slice { element, .. } => {
                let elem_ty = element.to_rust_type_static();
                out.push(format!("*const <{elem_ty} as FfiType>::CRepr"));
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
                        .is_some_and(|e| e.kind.is_handle());
                if is_borrowed_handle {
                    args.push(format!("FfiHandle::as_handle({})", p.name));
                } else {
                    let ty = tr.to_rust_type();
                    args.push(format!("<{ty} as FfiType>::into_c({})", p.name));
                }
            }
            ParamType::Slice { element, .. } => {
                if element.ref_kind != ffier_schema::RefKind::None {
                    // Ref elements (str, handles) were converted into a Vec
                    // by emit_slice_pre_bindings — use the converted Vec.
                    args.push(format!("__ffi_{}.as_ptr()", p.name));
                    args.push(format!("__ffi_{}.len()", p.name));
                } else {
                    // Non-ref elements (primitives) are layout-compatible,
                    // pass the slice data directly.
                    args.push(format!("{}.as_ptr()", p.name));
                    args.push(format!("{}.len()", p.name));
                }
            }
            ParamType::ImplTrait { .. } => {
                args.push(format!("{}.__into_raw_handle()", p.name));
            }
        }
    }
    args
}

/// Emit slice pre-bindings for ref-element slices (converting each element
/// via `into_c()` into a `Vec<CRepr>`). Non-ref element slices (primitives)
/// are layout-compatible and need no pre-binding.
fn emit_slice_pre_bindings(out: &mut String, params: &[ffier_schema::Param], indent: &str) {
    for p in params {
        if let ParamType::Slice { element, .. } = &p.param_type
            && element.ref_kind != ffier_schema::RefKind::None
        {
            let elem_ty = element.to_rust_type();
            writeln!(
                out,
                "{indent}let __ffi_{}: Vec<<{elem_ty} as FfiType>::CRepr> = {}.iter().map(|s| <{elem_ty} as FfiType>::into_c(*s)).collect();",
                p.name, p.name,
            )
            .unwrap();
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
    lib: &Library,
) {
    let sep = if args_str.is_empty() { "" } else { ", " };
    match ret {
        Return::Void => {
            writeln!(out, "{indent}unsafe {{ {ffi_name}({args_str}) }}").unwrap();
        }
        Return::ObjectArray { .. } => {
            writeln!(
                out,
                "{indent}ForeignSlice::from_raw(unsafe {{ {ffi_name}({args_str}) }})"
            )
            .unwrap();
        }
        Return::Value(tr) => {
            writeln!(
                out,
                "{indent}let __raw = unsafe {{ {ffi_name}({args_str}) }};"
            )
            .unwrap();
            let ty = tr.to_rust_type();
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
                "{indent}if __r == 0 {{ Ok(()) }} else {{ Err(unsafe {{ {err_type}::from_ffi(__r, __err) }}) }}"
            )
            .unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            c_convention,
        } => {
            use ffier_schema::CResultConvention;
            writeln!(
                out,
                "{indent}let mut __err: *mut core::ffi::c_void = core::ptr::null_mut();"
            )
            .unwrap();
            match c_convention {
                CResultConvention::HandleOrNull => {
                    let (error_result_fn, _) = find_error_dispatch_fns(lib);
                    writeln!(out, "{indent}let __raw = unsafe {{ {ffi_name}({args_str}{sep}&mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
                    writeln!(out, "{indent}if !__raw.is_null() {{").unwrap();
                    let ty = ok_tr.to_rust_type();
                    writeln!(
                        out,
                        "{indent}    Ok(unsafe {{ <{ty} as FfiType>::from_c(__raw) }})"
                    )
                    .unwrap();
                    writeln!(out, "{indent}}} else {{").unwrap();
                    writeln!(
                        out,
                        "{indent}    let __r = unsafe {{ {error_result_fn}(__err) }};"
                    )
                    .unwrap();
                    writeln!(
                        out,
                        "{indent}    Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                    )
                    .unwrap();
                    writeln!(out, "{indent}}}").unwrap();
                }
                CResultConvention::OutParam => {
                    let ty_static = ok_tr.to_rust_type_static();
                    writeln!(out, "{indent}let mut __result = core::mem::MaybeUninit::<<{ty_static} as FfiType>::CRepr>::uninit();").unwrap();
                    writeln!(out, "{indent}let __r = unsafe {{ {ffi_name}({args_str}{sep}__result.as_mut_ptr(), &mut __err as *mut *mut core::ffi::c_void) }};").unwrap();
                    writeln!(out, "{indent}if __r == 0 {{").unwrap();
                    let ty = ok_tr.to_rust_type();
                    writeln!(out, "{indent}    Ok(unsafe {{ <{ty} as FfiType>::from_c(__result.assume_init()) }})").unwrap();
                    writeln!(out, "{indent}}} else {{").unwrap();
                    writeln!(
                        out,
                        "{indent}    Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                    )
                    .unwrap();
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
        writeln!(out, "#[cfg(unix)]").unwrap();
        writeln!(out, "impl FfiType for OwnedFd {{").unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"int\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(out, "    fn into_c(self) -> {repr} {{ use std::os::unix::io::IntoRawFd; self.into_raw_fd() as {repr} }}").unwrap();
        writeln!(
            out,
            "    unsafe fn from_c(fd: {repr}) -> Self {{ unsafe {{ OwnedFd::from_raw_fd(fd as _) }} }}"
        )
        .unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Option<OwnedFd>
        writeln!(out, "#[cfg(unix)]").unwrap();
        writeln!(out, "impl FfiType for Option<OwnedFd> {{").unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"int\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(out, "    fn into_c(self) -> {repr} {{ use std::os::unix::io::IntoRawFd; match self {{ Some(fd) => fd.into_raw_fd() as {repr}, None => -1 }} }}").unwrap();
        writeln!(out, "    unsafe fn from_c(fd: {repr}) -> Self {{ if fd < 0 {{ None }} else {{ Some(unsafe {{ OwnedFd::from_raw_fd(fd as _) }}) }} }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    if let Some((_, entry)) = lib.blessed(Blessing::BorrowedFd) {
        let repr = fd_repr(entry, lib);
        writeln!(out, "#[cfg(unix)]").unwrap();
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
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Option<BorrowedFd>
        writeln!(out, "#[cfg(unix)]").unwrap();
        writeln!(out, "impl<'fd> FfiType for Option<BorrowedFd<'fd>> {{").unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"int\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(out, "    fn into_c(self) -> {repr} {{ match self {{ Some(fd) => fd.as_raw_fd() as {repr}, None => -1 }} }}").unwrap();
        writeln!(out, "    unsafe fn from_c(fd: {repr}) -> Self {{ if fd < 0 {{ None }} else {{ Some(unsafe {{ BorrowedFd::borrow_raw(fd as _) }}) }} }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    if let Some((_, entry)) = lib.blessed(Blessing::OwnedHandle) {
        let repr = fd_repr(entry, lib);
        writeln!(out, "#[cfg(windows)]").unwrap();
        writeln!(out, "impl FfiType for OwnedHandle {{").unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"void *\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(out, "    fn into_c(self) -> {repr} {{ use std::os::windows::io::IntoRawHandle; self.into_raw_handle() as {repr} }}").unwrap();
        writeln!(out, "    unsafe fn from_c(handle: {repr}) -> Self {{ unsafe {{ OwnedHandle::from_raw_handle(handle as _) }} }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        writeln!(out, "#[cfg(windows)]").unwrap();
        writeln!(out, "impl FfiType for Option<OwnedHandle> {{").unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"void *\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(out, "    fn into_c(self) -> {repr} {{ use std::os::windows::io::IntoRawHandle; self.map_or(core::ptr::null_mut(), |handle| handle.into_raw_handle() as {repr}) }}").unwrap();
        writeln!(out, "    unsafe fn from_c(handle: {repr}) -> Self {{ if handle.is_null() {{ None }} else {{ Some(unsafe {{ OwnedHandle::from_raw_handle(handle as _) }}) }} }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    if let Some((_, entry)) = lib.blessed(Blessing::BorrowedHandle) {
        let repr = fd_repr(entry, lib);
        writeln!(out, "#[cfg(windows)]").unwrap();
        writeln!(out, "impl<'handle> FfiType for BorrowedHandle<'handle> {{").unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"void *\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(out, "#[cfg(windows)]").unwrap();
        writeln!(
            out,
            "    fn into_c(self) -> {repr} {{ self.as_raw_handle() as {repr} }}"
        )
        .unwrap();
        writeln!(out, "    unsafe fn from_c(handle: {repr}) -> Self {{ unsafe {{ BorrowedHandle::borrow_raw(handle as _) }} }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        writeln!(out, "#[cfg(windows)]").unwrap();
        writeln!(
            out,
            "impl<'handle> FfiType for Option<BorrowedHandle<'handle>> {{"
        )
        .unwrap();
        writeln!(out, "    type CRepr = {repr}; const C_TYPE_NAME: &'static str = \"void *\"; const IS_HANDLE: bool = false;").unwrap();
        writeln!(out, "    fn into_c(self) -> {repr} {{ self.map_or(core::ptr::null_mut(), |handle| handle.as_raw_handle() as {repr}) }}").unwrap();
        writeln!(out, "    unsafe fn from_c(handle: {repr}) -> Self {{ if handle.is_null() {{ None }} else {{ Some(unsafe {{ BorrowedHandle::borrow_raw(handle as _) }}) }} }}").unwrap();
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
    emit_cfg_attr(out, en.cfg.as_deref());
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]").unwrap();
    writeln!(out, "#[repr({repr})]").unwrap();
    writeln!(out, "pub enum {} {{", en.name).unwrap();
    for v in &en.variants {
        writeln!(out, "    {} = {},", v.name, v.value).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiType impl
    emit_cfg_attr(out, en.cfg.as_deref());
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
    emit_cfg_attr(out, bf.cfg.as_deref());
    writeln!(out, "bitflags::bitflags! {{").unwrap();
    writeln!(out, "    #[repr(transparent)]").unwrap();
    writeln!(out, "    #[derive(Debug, Clone, Copy, PartialEq, Eq)]").unwrap();
    writeln!(out, "    pub struct {}: {repr} {{", bf.name).unwrap();
    for v in &bf.variants {
        writeln!(out, "        const {} = {};", v.name, v.value).unwrap();
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // FfiType impl
    emit_cfg_attr(out, bf.cfg.as_deref());
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
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ===========================================================================
// Free function generation
// ===========================================================================

fn emit_free_function(
    out: &mut String,
    f: &FreeFunction,
    lib: &Library,
    symbols: &mut Vec<SymbolInfo>,
) {
    // Extern declaration
    emit_extern_fns(out, &[extern_fn_from_free(f)], None, symbols);

    emit_cfg_attr(out, f.cfg.as_deref());
    for doc in &f.doc {
        let escaped = doc.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(out, "#[doc = \"{escaped}\"]").unwrap();
    }

    let param_sigs: Vec<String> = f.params.iter().map(format_param_sig).collect();
    let params_str = param_sigs.join(", ");

    let ret_type = match &f.ret {
        Return::Void => String::new(),
        Return::ObjectArray { element } => {
            format!(" -> ForeignSlice<{}>", element.type_name)
        }
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
    emit_call_preflight(out, &f.ffi_name, &f.params, "    ");

    emit_slice_pre_bindings(out, &f.params, "    ");
    let ffi_args = build_ffi_param_args(&f.params, lib);
    let args_str = ffi_args.join(", ");
    let sep = if args_str.is_empty() { "" } else { ", " };

    match &f.ret {
        Return::Void => {
            writeln!(out, "    unsafe {{ {}({args_str}) }}", f.ffi_name).unwrap();
        }
        Return::ObjectArray { .. } => {
            writeln!(
                out,
                "    ForeignSlice::from_raw(unsafe {{ {}({args_str}) }})",
                f.ffi_name
            )
            .unwrap();
        }
        Return::Value(tr) => {
            writeln!(
                out,
                "    let __raw = unsafe {{ {}({args_str}) }};",
                f.ffi_name
            )
            .unwrap();
            let ty = tr.to_rust_type();
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
                "    if __r == 0 {{ Ok(()) }} else {{ Err(unsafe {{ {err_type}::from_ffi(__r, __err) }}) }}"
            )
            .unwrap();
        }
        Return::Result {
            ok: Some(ok_tr),
            err_type,
            c_convention,
        } => {
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
                let ty = ok_tr.to_rust_type();
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
                writeln!(
                    out,
                    "        Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                )
                .unwrap();
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
                let ty = ok_tr.to_rust_type();
                writeln!(
                    out,
                    "        Ok(unsafe {{ <{ty} as FfiType>::from_c(__out.assume_init()) }})"
                )
                .unwrap();
                writeln!(out, "    }} else {{").unwrap();
                writeln!(
                    out,
                    "        Err(unsafe {{ {err_type}::from_ffi(__r, __err) }})"
                )
                .unwrap();
                writeln!(out, "    }}").unwrap();
            }
        }
    }

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn format_param_sig(p: &ffier_schema::Param) -> String {
    match &p.param_type {
        ParamType::Regular(tr) => format!("{}: {}", p.name, tr.to_rust_type()),
        ParamType::Slice { element, .. } => {
            format!("{}: &[{}]", p.name, element.to_rust_type())
        }
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
