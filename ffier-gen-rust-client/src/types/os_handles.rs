//! Platform file descriptor and Windows handle conversions.

use std::fmt::Write;

use ffier_schema::Library;

pub(crate) fn emit_blessed_fd_impls(out: &mut String, lib: &Library) {
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
