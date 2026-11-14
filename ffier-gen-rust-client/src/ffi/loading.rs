//! Emit symbol identities, explicit loading, and loading policy.

use std::fmt::Write;

use crate::cfg::{emit_cfg_attr, uncovered_guard};

use super::SymbolInfo;

/// Convert a C symbol name like `ft_widget_new` to a PascalCase enum variant
/// like `FtWidgetNew`.
pub(super) fn symbol_to_variant(c_name: &str) -> String {
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
pub(crate) fn emit_symbol_api(
    out: &mut String,
    symbols: &[SymbolInfo],
    handle_allocator: Option<&str>,
) {
    out.push_str("/// FFI symbol loading and client configuration.\npub mod ffi {\n");

    // Resolve ABI types in the library API's scope, separate from loader types
    // such as Symbol and LoadError, which may also be user-defined type names.
    out.push_str(
        "#[cfg(not(feature = \"ffier-strong\"))]\npub(super) mod __symbols {\n#[allow(unused_imports)]\nuse super::super::*;\n",
    );
    for symbol in symbols {
        emit_cfg_attr(out, symbol.cfg.as_ref());
        writeln!(
            out,
            "pub(in super::super) static {}: std::sync::OnceLock<{}> = std::sync::OnceLock::new();",
            symbol.upper, symbol.fn_ptr_type
        )
        .unwrap();
    }
    out.push_str("}\n");

    // Cfg-exclusive variants of one trait method share a C symbol name, so the
    // enum and its name mapping key off unique names; the loader keeps one arm
    // per variant.
    let mut names: Vec<&str> = Vec::new();
    for s in symbols {
        if !names.contains(&s.c_name.as_str()) {
            names.push(&s.c_name);
        }
    }

    out.push_str(
        r#"
/// An FFI symbol, with the same identity in both linking modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::enum_variant_names)] // Preserve the C library prefix in symbol identities.
pub enum Symbol {
"#,
    );
    for name in &names {
        let variant = symbol_to_variant(name);
        writeln!(out, "    /// `{name}`\n    {variant},").unwrap();
    }
    out.push_str(
        r#"}
impl Symbol {
    /// The C symbol name.
    pub fn c_name(self) -> &'static str {
        match self {
"#,
    );
    for name in &names {
        let variant = symbol_to_variant(name);
        writeln!(out, "            Symbol::{variant} => \"{name}\",").unwrap();
    }
    out.push_str(
        r#"        }
    }

    #[cfg(not(feature = "ffier-strong"))]
    fn __lookup<F: Copy>(self, lib: &__FfierNativeLibrary) -> Result<F, libloading::Error> {
        let name: &[u8] = match self {
"#,
    );
    for name in &names {
        let variant = symbol_to_variant(name);
        writeln!(out, "            Symbol::{variant} => b\"{name}\\0\",").unwrap();
    }
    out.push_str(
        r#"        };
        unsafe { lib.get::<F>(name).map(|function| *function) }
    }

    /// Probe the configured lookup source without initializing the symbol cache.
    /// With `ffier-strong`, always returns true: availability reflects the linking
    /// contract, not a runtime probe. Missing symbols are the linker's responsibility.
    pub fn is_available(self) -> bool {
        #[cfg(feature = "ffier-strong")]
        {
            true
        }
        #[cfg(not(feature = "ffier-strong"))]
        {
            __ffier_with_library(|lib| self.__lookup::<*const ()>(lib).is_ok()).unwrap_or(false)
        }
    }

    #[cfg(not(feature = "ffier-strong"))]
    fn __load(self, lib: &__FfierNativeLibrary) -> Result<(), LoadError> {
        match self {
"#,
    );
    for name in &names {
        let variant = symbol_to_variant(name);
        let variants: Vec<&SymbolInfo> = symbols.iter().filter(|s| s.c_name == *name).collect();
        writeln!(out, "            Symbol::{variant} => {{").unwrap();
        // One block per variant, each under its own predicate. A variant with no
        // predicate emits its body directly.
        for s in &variants {
            let indent = if s.cfg.is_some() {
                emit_cfg_attr(out, s.cfg.as_ref());
                writeln!(out, "                {{").unwrap();
                "                    "
            } else {
                "                "
            };
            writeln!(out, "{indent}if __symbols::{}.get().is_none() {{", s.upper).unwrap();
            writeln!(out, "{indent}    let f = self.__lookup(lib)?;").unwrap();
            writeln!(out, "{indent}    let _ = __symbols::{}.set(f);", s.upper).unwrap();
            writeln!(out, "{indent}}}").unwrap();
            if s.cfg.is_some() {
                writeln!(out, "                }}").unwrap();
            }
        }
        // Wherever no variant applies the symbol still has to exist, so probing
        // it keeps `require()` honest.
        if let Some(guard) = uncovered_guard(variants.iter().map(|s| s.cfg.as_ref())) {
            writeln!(out, "                #[cfg({guard})]").unwrap();
            writeln!(
                out,
                "                {{ self.__lookup::<*const ()>(lib)?; }}"
            )
            .unwrap();
        }
        writeln!(out, "            }}").unwrap();
    }
    out.push_str(
        r#"        }
        Ok(())
    }
}

/// Controls how calls to unloaded symbols are handled. Defaults to `Lazy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadPolicy {
    /// Require explicit loading with `require()`; unloaded calls panic.
    Explicit,
    /// Resolve and cache symbols on first use; missing symbols panic.
    Lazy,
}

/// The client's lookup source or loading policy has already been fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetLibraryError;
impl std::fmt::Display for SetLibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("library lookup configuration is already initialized")
    }
}
impl std::error::Error for SetLibraryError {}

/// Set this generated client's lookup source once.
/// `None` selects the process-global namespace on Unix or the current executable
/// on Windows. `Some(handle)` borrows an existing `dlopen` handle or Windows
/// `HMODULE`. Windows clients should pass their DLL handle explicitly. Probes,
/// explicit loads, and lazy loads all use this source. Reconfiguration is rejected,
/// even before a symbol is loaded.
/// Without this call, the first probe, load, or unloaded call fixes the source
/// to that platform default.
/// With `ffier-strong`, this is a successful no-op and ignores the argument.
///
/// # Safety
/// An explicit handle must be valid and refer to the library matching these
/// bindings. The caller retains ownership and must keep it loaded for the
/// remaining lifetime of the program. In `ffier-strong` mode the handle is ignored.
pub unsafe fn set_library_handle(
    lib_handle: Option<core::ptr::NonNull<core::ffi::c_void>>,
) -> Result<(), SetLibraryError> {
    #[cfg(feature = "ffier-strong")]
    {
        let _ = lib_handle;
        Ok(())
    }
    #[cfg(not(feature = "ffier-strong"))]
    {
        let library = lib_handle.map(|handle| std::mem::ManuallyDrop::new(unsafe {
            __FfierNativeLibrary::from_raw(handle.as_ptr() as _)
        }));
        __FFIER_LIBRARY.set(library).map_err(|_| SetLibraryError)
    }
}

/// Select how calls to unloaded symbols are handled, once per client.
/// Defaults to `LoadPolicy::Lazy`. `LoadPolicy::Explicit` requires loading with
/// `require()` before calling symbols. Reconfiguration is rejected, including
/// after the policy has already been applied to a call.
/// With `ffier-strong`, this is a successful no-op and ignores the argument.
pub fn set_load_policy(load_policy: LoadPolicy) -> Result<(), SetLibraryError> {
    #[cfg(feature = "ffier-strong")]
    {
        let _ = load_policy;
        Ok(())
    }
    #[cfg(not(feature = "ffier-strong"))]
    {
        __FFIER_POLICY.set(load_policy).map_err(|_| SetLibraryError)
    }
}

#[cfg(all(not(feature = "ffier-strong"), unix))]
use libloading::os::unix::Library as __FfierNativeLibrary;
#[cfg(all(not(feature = "ffier-strong"), windows))]
use libloading::os::windows::Library as __FfierNativeLibrary;
#[cfg(not(feature = "ffier-strong"))]
type __FfierLibrary = Option<std::mem::ManuallyDrop<__FfierNativeLibrary>>;
#[cfg(not(feature = "ffier-strong"))]
static __FFIER_LIBRARY: std::sync::OnceLock<__FfierLibrary> = std::sync::OnceLock::new();
#[cfg(not(feature = "ffier-strong"))]
static __FFIER_POLICY: std::sync::OnceLock<LoadPolicy> = std::sync::OnceLock::new();

#[cfg(not(feature = "ffier-strong"))]
fn __ffier_with_library<R>(
    operation: impl FnOnce(&__FfierNativeLibrary) -> R,
) -> Result<R, libloading::Error> {
    let result = match __FFIER_LIBRARY.get_or_init(|| None) {
        Some(library) => operation(library),
        None => {
            #[cfg(unix)]
            let library = __FfierNativeLibrary::this();
            #[cfg(windows)]
            let library = __FfierNativeLibrary::this()?;
            operation(&library)
        }
    };
    Ok(result)
}

#[cfg(not(feature = "ffier-strong"))]
pub(super) fn __ffier_symbol<F>(symbol: Symbol, slot: &std::sync::OnceLock<F>) -> &F {
    slot.get().unwrap_or_else(|| {
        match __FFIER_POLICY.get_or_init(|| LoadPolicy::Lazy) {
            LoadPolicy::Explicit => panic!(
                "symbol `{}` not loaded; call ffi::require() first", symbol.c_name()),
            LoadPolicy::Lazy => require(&[symbol]).unwrap_or_else(|error|
                panic!("failed to load symbol `{}`: {error}", symbol.c_name())),
        }
        slot.get().expect("loaded symbol cache is empty")
    })
}

/// A symbol loading failure. Strong linking has no loading-failure states.
#[derive(Debug)]
pub enum LoadError {
    /// The original dynamic loader error, without allocation or conversion.
    #[cfg(not(feature = "ffier-strong"))]
    Loading(libloading::Error),
}
impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        match *self {
            #[cfg(not(feature = "ffier-strong"))]
            Self::Loading(ref error) => std::fmt::Display::fmt(error, f),
        }
    }
}
impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            #[cfg(not(feature = "ffier-strong"))]
            Self::Loading(ref error) => Some(error),
        }
    }
}
#[cfg(not(feature = "ffier-strong"))]
impl From<libloading::Error> for LoadError {
    fn from(error: libloading::Error) -> Self { Self::Loading(error) }
}

/// Load requested symbols from this client's configured source, skipping
/// initialized caches and returning the first failure. Without a setter call,
/// uses the process-global namespace on Unix or the current executable on Windows.
/// A client with any exported vtable also loads the library's shared handle allocator.
/// With `ffier-strong`, ignores the symbols and always succeeds.
pub fn require(symbols: &[Symbol]) -> Result<(), LoadError> {
    #[cfg(feature = "ffier-strong")]
    {
        let _ = symbols;
        Ok(())
    }
    #[cfg(not(feature = "ffier-strong"))]
    {
        __ffier_with_library(|lib| {
"#,
    );
    if let Some(name) = handle_allocator {
        let variant = symbol_to_variant(name);
        writeln!(out, "            Symbol::{variant}.__load(lib)?;").unwrap();
    }
    out.push_str(
        r#"
            for &symbol in symbols {
                symbol.__load(lib)?;
            }
            Ok(())
        })?
    }
}
}
"#,
    );
}
