use std::collections::HashSet;
use std::fmt::Write;

use ffier_schema::{Cfg, ExportedType, Library, Method, Receiver};

use super::{emit_cfg_attr, emit_method_wrapper, forwarder_cfg, lifetime_generics};

/// Emit lifetime-carrying views over generated handle wrappers.
pub(super) fn emit(out: &mut String) {
    out.push_str(
        r#"
/// Shared access to a handle whose value remains owned elsewhere.
///
/// Dereferencing permits shared methods without moving the handle out.
/// Dropping an imported view releases its borrowed shell, not its value.
pub struct Ref<'a, T: FfiHandle> {
    value: __RefValue<'a, T>,
}

enum __RefValue<'a, T> {
    Borrowed(&'a T),
    Imported(T),
}

impl<'a, T: FfiHandle> Ref<'a, T> {
    /// # Safety
    /// `raw` must be a valid handle for the generated wrapper `T`.
    /// Its value must permit shared access for `'a`.
    /// This view must uniquely own the borrowed shell.
    #[doc(hidden)]
    pub unsafe fn __from_raw(raw: *mut core::ffi::c_void) -> Self {
        Self { value: __RefValue::Imported(unsafe { T::__from_raw(raw) }) }
    }

}

impl<'a, T: FfiHandle> From<&'a T> for Ref<'a, T> {
    fn from(value: &'a T) -> Self {
        Self { value: __RefValue::Borrowed(value) }
    }
}

impl<'a, T: FfiHandle> From<&'a mut T> for Ref<'a, T> {
    fn from(value: &'a mut T) -> Self {
        Self::from(&*value)
    }
}

impl<'a, T: FfiHandle> From<&'a Ref<'_, T>> for Ref<'a, T> {
    fn from(value: &'a Ref<'_, T>) -> Self {
        Self::from(&**value)
    }
}

impl<'a, T: FfiHandle> From<&'a RefMut<'_, T>> for Ref<'a, T> {
    fn from(value: &'a RefMut<'_, T>) -> Self {
        Self::from(&**value)
    }
}

impl<T: FfiHandle> core::ops::Deref for Ref<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        match &self.value {
            __RefValue::Borrowed(value) => value,
            __RefValue::Imported(value) => value,
        }
    }
}

impl<T: FfiHandle + core::fmt::Debug> core::fmt::Debug for Ref<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Ref").field(&**self).finish()
    }
}

/// Exclusive access to a handle whose value remains owned elsewhere.
///
/// Shared methods are available through `Deref`; generated mutable methods
/// reborrow this view. No `DerefMut` is provided: replacing the client wrapper
/// would move a borrowed handle out without moving the library's value.
pub struct RefMut<'a, T: FfiHandle> {
    value: __RefMutValue<'a, T>,
}

enum __RefMutValue<'a, T> {
    Borrowed(&'a mut T),
    Imported(T),
}

impl<'a, T: FfiHandle> RefMut<'a, T> {
    /// # Safety
    /// `raw` must be a valid handle for the generated wrapper `T`.
    /// Its value must permit exclusive access for `'a`.
    /// This view must uniquely own the borrowed shell.
    #[doc(hidden)]
    pub unsafe fn __from_raw(raw: *mut core::ffi::c_void) -> Self {
        Self { value: __RefMutValue::Imported(unsafe { T::__from_raw(raw) }) }
    }

    fn __as_raw(&self) -> *mut core::ffi::c_void {
        unsafe { T::as_handle(self) }
    }

    /// # Safety
    /// The reference must remain inside generated code and must never permit
    /// moving or replacing the wrapped handle.
    unsafe fn __get_mut(&mut self) -> &mut T {
        match &mut self.value {
            __RefMutValue::Borrowed(value) => value,
            __RefMutValue::Imported(value) => value,
        }
    }

}

impl<'a, T: FfiHandle> From<&'a mut T> for RefMut<'a, T> {
    fn from(value: &'a mut T) -> Self {
        Self { value: __RefMutValue::Borrowed(value) }
    }
}

impl<'a, T: FfiHandle> From<&'a mut RefMut<'_, T>> for RefMut<'a, T> {
    fn from(value: &'a mut RefMut<'_, T>) -> Self {
        // SAFETY: the new view keeps the original borrowed and never exposes &mut T.
        Self::from(unsafe { value.__get_mut() })
    }
}

impl<T: FfiHandle> core::ops::Deref for RefMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        match &self.value {
            __RefMutValue::Borrowed(value) => value,
            __RefMutValue::Imported(value) => value,
        }
    }
}

impl<T: FfiHandle + core::fmt::Debug> core::fmt::Debug for RefMut<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("RefMut").field(&**self).finish()
    }
}

"#,
    );
}

/// Preserve each native impl's lifetime context when forwarding mutable methods.
pub(super) fn emit_methods(out: &mut String, ty: &ExportedType, library: &Library) {
    let type_lifetimes: Vec<_> = library
        .type_entry(&ty.name)
        .unwrap()
        .lifetime_params
        .iter()
        .map(|lifetime| lifetime.name.clone())
        .collect();
    let methods: Vec<_> = ty
        .methods
        .iter()
        .filter(|method| method.receiver == Receiver::Mut)
        .cloned()
        .collect();
    emit_method_group(
        out,
        ty,
        library,
        &methods,
        &type_lifetimes,
        &type_lifetimes,
        None,
    );

    let inherent_names: HashSet<_> = ty.methods.iter().map(|method| &method.name).collect();
    let mut trait_groups = Vec::new();
    for implementation in library
        .trait_impls
        .iter()
        .filter(|implementation| implementation.struct_name == ty.name)
    {
        let mut methods = implementation.methods.clone();
        if let Some(definition) = library
            .traits
            .iter()
            .find(|definition| definition.name == implementation.trait_name)
        {
            for method in &definition.methods {
                if method
                    .trait_definition
                    .as_ref()
                    .is_some_and(|definition| definition.has_default)
                    && let Some(cfg) = forwarder_cfg(method, implementation)
                {
                    let mut method = method.clone();
                    method.cfg = cfg;
                    methods.push(method);
                }
            }
        }
        methods.retain(|method| {
            method.receiver == Receiver::Mut && !inherent_names.contains(&method.name)
        });
        trait_groups.push((implementation, methods));
    }
    for (implementation, methods) in &trait_groups {
        // Competing trait methods need qualified calls in native Rust too.
        let methods: Vec<_> = methods
            .iter()
            .filter(|method| {
                !trait_groups.iter().any(|(other, methods)| {
                    other.trait_name != implementation.trait_name
                        && methods.iter().any(|other| other.name == method.name)
                })
            })
            .cloned()
            .collect();
        emit_method_group(
            out,
            ty,
            library,
            &methods,
            &implementation.lifetimes,
            &implementation.struct_lifetime_args,
            implementation.cfg.as_ref(),
        );
    }
}

fn emit_method_group(
    out: &mut String,
    ty: &ExportedType,
    library: &Library,
    methods: &[Method],
    impl_lifetimes: &[String],
    type_lifetimes: &[String],
    cfg: Option<&Cfg>,
) {
    if methods.is_empty() {
        return;
    }
    let used_lifetimes: HashSet<_> = impl_lifetimes
        .iter()
        .chain(type_lifetimes)
        .chain(methods.iter().flat_map(|method| &method.method_lifetimes))
        .map(String::as_str)
        .collect();
    let view_lifetime = (0..)
        .map(|index| format!("__ffier_view_{index}"))
        .find(|name| !used_lifetimes.contains(name.as_str()))
        .unwrap();
    let generics = lifetime_generics(
        &std::iter::once(view_lifetime.clone())
            .chain(impl_lifetimes.iter().cloned())
            .collect::<Vec<_>>(),
    );
    let type_arguments = lifetime_generics(type_lifetimes);
    emit_cfg_attr(out, ty.cfg.as_ref());
    emit_cfg_attr(out, cfg);
    writeln!(
        out,
        "impl{generics} RefMut<'{view_lifetime}, {}{type_arguments}> {{",
        ty.name
    )
    .unwrap();
    for method in methods {
        emit_method_wrapper(
            out,
            method,
            false,
            !type_lifetimes.is_empty(),
            library,
            true,
        );
    }
    writeln!(out, "}}\n").unwrap();
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{Value, json};

    use super::*;

    fn library(type_lifetimes: &[&str], implementation: Value) -> Library {
        Library::from_json(
            &json!({
                "prefix": "probe",
                "type_registry": {
                    "Widget": {"kind": {"handle": {"c_name": "ProbeWidget"}}, "type_tag": 1,
                        "lifetime_params": type_lifetimes.iter().map(|name| json!({"name": name, "variance": "covariant"})).collect::<Vec<_>>()},
                    "str": {"kind": {"string": {"c_name": "ProbeStr"}}},
                    "i32": {"kind": {"primitive": {"c_type": "int32_t"}}}
                },
                "exported_types": [{"name": "Widget", "destroy_ffi_name": "probe_widget_destroy", "is_builder_type": false, "methods": []}],
                "errors": [], "traits": [], "trait_impls": [implementation]
            })
            .to_string(),
        )
        .unwrap()
    }

    fn set_method() -> Value {
        json!({
            "name": "set", "ffi_name": "probe_set", "receiver": "mut",
            "params": [{"name": "value", "param_kind": "regular", "type": "str", "ref_kind": "shared", "ref_lifetime": "argument"}],
            "ret": {"return_kind": "void"}
        })
    }

    fn compile(source: &str) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "ffier-borrow-methods-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("probe.rs");
        std::fs::write(&path, source).unwrap();
        let output = Command::new("rustc")
            .args(["--edition=2024", "--crate-type=lib", "--emit=metadata"])
            .args(["--cfg", "feature=\"ffier-strong\""])
            .arg(&path)
            .arg("--out-dir")
            .arg(&directory)
            .output()
            .unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        assert!(
            output.status.success(),
            "generated bindings failed to compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    const PRELUDE: &str = r#"
#![allow(dead_code)]
trait FfiHandle {}
struct RefMut<'a, T: FfiHandle>(&'a mut T);
impl<T: FfiHandle> RefMut<'_, T> {
    fn __as_raw(&self) -> *mut core::ffi::c_void { core::ptr::null_mut() }
}
trait FfiType { type CRepr; fn into_c(self) -> Self::CRepr; }
impl<'a> FfiType for &'a str { type CRepr = &'a str; fn into_c(self) -> Self { self } }
impl FfiType for i32 { type CRepr = i32; fn into_c(self) -> Self { self } }
unsafe fn probe_set(_: *mut core::ffi::c_void, _: &str) {}
unsafe fn probe_set_number(_: *mut core::ffi::c_void, _: i32) {}
unsafe fn probe_reset(_: *mut core::ffi::c_void) {}
"#;

    #[test]
    fn mutable_trait_methods_compile_with_native_impl_lifetimes() {
        for (type_lifetimes, impl_lifetimes, struct_arguments, declaration, exercise_type) in [
            (vec![], vec!["argument"], vec![], "struct Widget;", "Widget"),
            (
                vec!["native"],
                vec!["renamed", "argument"],
                vec!["renamed"],
                "struct Widget<'native>(&'native ());",
                "Widget<'owner>",
            ),
            (
                vec!["native"],
                vec!["argument"],
                vec!["static"],
                "struct Widget<'native>(&'native ());",
                "Widget<'static>",
            ),
        ] {
            let library = library(
                &type_lifetimes,
                json!({
                    "trait_name": "Set", "struct_name": "Widget",
                    "lifetimes": impl_lifetimes, "trait_lifetime_args": ["argument"],
                    "struct_lifetime_args": struct_arguments, "methods": [set_method()]
                }),
            );
            let mut source =
                format!("{PRELUDE}\n{declaration}\nimpl FfiHandle for {exercise_type} {{}}\n");
            if !type_lifetimes.is_empty() {
                source = format!("{PRELUDE}\n{declaration}\nimpl FfiHandle for Widget<'_> {{}}\n");
            }
            emit_methods(&mut source, &library.exported_types[0], &library);
            write!(source, "fn exercise<'owner, 'input>(mut value: RefMut<'owner, {exercise_type}>, input: &'input str) {{ value.set(input); }}").unwrap();
            compile(&source);
        }
    }

    #[test]
    fn mutable_methods_keep_inherent_precedence_and_trait_defaults() {
        let mut library = library(
            &[],
            json!({
                "trait_name": "Set", "struct_name": "Widget", "lifetimes": ["argument"],
                "trait_lifetime_args": ["argument"], "methods": [set_method()]
            }),
        );
        library.exported_types[0].methods.push(
            serde_json::from_value(json!({
                "name": "set", "ffi_name": "probe_set_number", "receiver": "mut",
                "params": [{"name": "value", "param_kind": "regular", "type": "i32"}],
                "ret": {"return_kind": "void"}
            }))
            .unwrap(),
        );
        library.trait_impls.push(
            serde_json::from_value(json!({
                "trait_name": "Reset", "struct_name": "Widget", "methods": []
            }))
            .unwrap(),
        );
        library.traits.push(serde_json::from_value(json!({
            "name": "Reset", "destroy_ffi_name": "probe_reset_destroy",
            "type_tag_constant": "PROBE_RESET_TYPE_TAG", "vtable_struct_c_name": "ProbeResetVtable",
            "wrapper_c_name": "ProbeVtableReset", "vtable_struct_name": "ResetVtable", "wrapper_name": "VtableReset",
            "methods": [{"name": "reset", "ffi_name": "probe_reset", "receiver": "mut", "params": [],
                "ret": {"return_kind": "void"}, "trait_definition": {"index": 0, "has_default": true}}],
            "own_method_count": 1, "num_slots": 1
        })).unwrap());
        let mut source = format!("{PRELUDE}\nstruct Widget;\nimpl FfiHandle for Widget {{}}\n");
        emit_methods(&mut source, &library.exported_types[0], &library);
        source.push_str(
            "fn exercise(mut value: RefMut<'_, Widget>) { value.set(1); value.reset(); }",
        );
        compile(&source);
    }

    #[test]
    fn foreign_free_returns_keep_reference_signatures() {
        let library = Library::from_json(
            &json!({
                "prefix": "probe", "exported_types": [], "errors": [], "trait_impls": [],
                "type_registry": {
                    "Foreign": {"kind": {"foreign_handle": {"foreign_crate": "foreign", "c_name": "ProbeForeign"}}},
                    "ErrorTrait": {"kind": {"trait": {"c_name": "ProbeError"}}, "bless": "error_trait"}
                },
                "traits": [{
                    "name": "ErrorTrait", "destroy_ffi_name": "probe_error_destroy",
                    "type_tag_constant": "PROBE_ERROR_TYPE_TAG", "vtable_struct_c_name": "ProbeErrorVtable",
                    "wrapper_c_name": "ProbeVtableError", "vtable_struct_name": "ErrorVtable", "wrapper_name": "VtableError",
                    "methods": [{"name": "result", "ffi_name": "probe_error_result", "receiver": "ref", "params": [],
                        "ret": {"return_kind": "void"}}], "own_method_count": 1, "num_slots": 1
                }]
            })
            .to_string(),
        )
        .unwrap();
        let foreign = json!({"type": "Foreign", "ref_kind": "shared", "ref_lifetime": "static"});
        for (ret, expected) in [
            (
                json!({"return_kind": "value", "type": "Foreign", "ref_kind": "shared", "ref_lifetime": "static"}),
                "&'static Foreign",
            ),
            (
                json!({"return_kind": "result", "ok": foreign, "err_type": "TestError", "c_convention": "handle_or_null"}),
                "Result<&'static Foreign, TestError>",
            ),
        ] {
            let function = serde_json::from_value(json!({
                "name": "borrow_foreign", "ffi_name": "probe_borrow_foreign", "params": [], "ret": ret
            }))
            .unwrap();
            let mut source = String::from(
                r#"
#![allow(dead_code)]
pub trait FfiHandle {
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void;
    unsafe fn __from_raw(raw: *mut core::ffi::c_void) -> Self;
}
trait FfiType {
    type CRepr;
    unsafe fn from_c(raw: Self::CRepr) -> Self;
}
struct Foreign;
impl FfiHandle for Foreign {
    unsafe fn as_handle(&self) -> *mut core::ffi::c_void { core::ptr::null_mut() }
    unsafe fn __from_raw(_: *mut core::ffi::c_void) -> Self { Self }
}
impl FfiType for &Foreign {
    type CRepr = *mut core::ffi::c_void;
    unsafe fn from_c(_: Self::CRepr) -> Self { unimplemented!() }
}
struct TestError;
impl TestError {
    unsafe fn from_ffi(_: u32, _: *mut core::ffi::c_void) -> Self { Self }
}
unsafe fn probe_error_result(_: *mut core::ffi::c_void) -> u32 { 1 }
mod foreign { pub use super::{Ref, RefMut}; }
"#,
            );
            emit(&mut source);
            super::super::emit_free_function(&mut source, &function, &library, &mut Vec::new());
            writeln!(
                source,
                "fn exercise() {{ let _: {expected} = borrow_foreign(); }}"
            )
            .unwrap();
            compile(&source);
        }
    }
}
