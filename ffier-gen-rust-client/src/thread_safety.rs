//! Thread-safety declarations for generated client handles and traits.

use std::fmt::Write;

use ffier_schema::{Cfg, Library, ThreadSafety};

use crate::cfg::emit_cfg_attr;

pub(crate) fn emit_thread_safety_impls(
    out: &mut String,
    name: &str,
    generics: &str,
    thread_safety: ThreadSafety,
    cfg: Option<&Cfg>,
) {
    for bound in thread_safety.rust_bounds() {
        emit_cfg_attr(out, cfg);
        writeln!(
            out,
            "// SAFETY: library verifies this guarantee; unsafe imports must preserve it."
        )
        .unwrap();
        writeln!(
            out,
            "unsafe impl{generics} {bound} for {name}{generics} {{}}"
        )
        .unwrap();
    }
}

pub(crate) fn trait_thread_safety(lib: &Library, name: &str) -> ThreadSafety {
    lib.traits
        .iter()
        .find(|tr| tr.name == name)
        .map_or(ThreadSafety::default(), |tr| tr.thread_safety)
}
