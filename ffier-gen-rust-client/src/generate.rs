//! Coordinate complete Rust client generation from a library schema.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use ffier_schema::{Library, Method, TypeKind};

use crate::error::{emit_error, emit_error_trait_externs};
use crate::ffi::{emit_extern_fns, emit_symbol_api, extern_fn_private};
use crate::ffi_traits::{C_REPR_ALIAS, TOKEN};
use crate::formatting::rustfmt;
use crate::functions::emit_free_function;
use crate::traits::{emit_implementable_trait, emit_trait_impl};
use crate::types::{emit_bitflags_type, emit_enum_type, emit_exported_type, emit_value_structs};

/// Generate one Rust client source supporting both Unix and Windows linking modes.
/// The consuming crate selects the implementation with its `ffier-dynlookup` or
/// `ffier-strong` feature, one of which is required. `ffier-strong` takes
/// precedence when both are enabled.
pub fn generate(lib: &Library) -> String {
    let mut out = String::new();
    out.push_str("#[cfg(not(any(feature = \"ffier-dynlookup\", feature = \"ffier-strong\")))]\ncompile_error!(\"generated ffier clients require the `ffier-dynlookup` or `ffier-strong` feature\");\n\n");
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
        // Generated signatures spell paths as bare `Path`.
        if lib.blessed(Blessing::Path).is_some() {
            writeln!(out, "use std::path::Path;").unwrap();
        }
        let fd_blessing_symbols: &[(Blessing, &[&str])] = &[
            (Blessing::RawFd, &["RawFd"]),
            (Blessing::BorrowedFd, &["BorrowedFd"]),
            (Blessing::OwnedFd, &["OwnedFd"]),
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
            // The raw type is registered as the alias target of the owned
            // and borrowed types even when no signature names it.
            writeln!(out, "#[allow(unused_imports)]").unwrap();
            writeln!(out, "use std::os::unix::io::{{{list}}};").unwrap();
            writeln!(out).unwrap();
        }

        let handle_blessing_symbols: &[(Blessing, &[&str])] = &[
            (Blessing::RawHandle, &["RawHandle"]),
            (Blessing::BorrowedHandle, &["BorrowedHandle"]),
            (Blessing::OwnedHandle, &["OwnedHandle"]),
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
            // The raw type is registered as the alias target of the owned
            // and borrowed types even when no signature names it.
            writeln!(out, "#[allow(unused_imports)]").unwrap();
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

    // The token for the shared `ffier` conversion traits; see `ffi_traits`.
    writeln!(
        out,
        "/// This client's token for the `ffier` conversion traits."
    )
    .unwrap();
    writeln!(out, "#[doc(hidden)]").unwrap();
    writeln!(out, "pub struct {TOKEN};").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// The C representation of `T`.").unwrap();
    writeln!(out, "#[doc(hidden)]").unwrap();
    writeln!(
        out,
        "pub type {C_REPR_ALIAS}<T> = <T as ffier::FfiType<{TOKEN}>>::CRepr;"
    )
    .unwrap();
    writeln!(out).unwrap();

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
