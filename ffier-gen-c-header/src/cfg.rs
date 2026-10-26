use ffier_schema::Cfg;

fn macro_part(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Constant-fold a predicate, mirroring Rust's `cfg` semantics: `all()` and
/// `cfg(true)` are true, `any()` and `cfg(false)` are false. Mutually exclusive
/// alternatives such as `any(A, not(A))` cover every configuration. Returns
/// `None` when the predicate depends on how the library is built.
fn constant(cfg: &Cfg) -> Option<bool> {
    match cfg {
        Cfg::All(items) | Cfg::Any(items) => {
            let all = matches!(cfg, Cfg::All(_));
            // A single dominating operand decides the combinator.
            if items.iter().any(|item| constant(item) == Some(!all)) {
                return Some(!all);
            }
            if items.iter().all(|item| constant(item).is_some()) {
                return Some(all);
            }
            let complementary = items.iter().any(|item| {
                let negated = Cfg::Not(Box::new(item.clone()));
                items.contains(&negated)
            });
            complementary.then_some(!all)
        }
        Cfg::Not(item) => constant(item).map(|value| !value),
        Cfg::Flag(_) | Cfg::Value { .. } => None,
    }
}

/// Translate target predicates using compiler predefined macros. Other cfg
/// names have an explicit FFIER_CFG_* override for the C consumer.
pub fn expression(cfg: &Cfg) -> String {
    if let Some(value) = constant(cfg) {
        return if value { "1" } else { "0" }.into();
    }
    match cfg {
        Cfg::Flag(key) => match key.as_str() {
            "windows" => "defined(_WIN32)".into(),
            "unix" => "(defined(__unix__) || defined(__APPLE__))".into(),
            _ => format!("defined(FFIER_CFG_{})", macro_part(key)),
        },
        Cfg::Value { key, value } => {
            match (key.as_str(), value.as_str()) {
                ("target_os" | "target_family", "windows") => return "defined(_WIN32)".into(),
                ("target_family", "unix") => return expression(&Cfg::Flag("unix".into())),
                ("target_os", "linux") => {
                    return "(defined(__linux__) && !defined(__ANDROID__))".into();
                }
                ("target_os", "macos") => {
                    return "(defined(__APPLE__) && TARGET_OS_OSX)".into();
                }
                ("target_os", "freebsd") => return "defined(__FreeBSD__)".into(),
                ("target_os", "openbsd") => return "defined(__OpenBSD__)".into(),
                ("target_os", "netbsd") => return "defined(__NetBSD__)".into(),
                ("target_os", "android") => return "defined(__ANDROID__)".into(),
                ("target_arch", "x86_64") => {
                    return "(defined(__x86_64__) || defined(_M_X64))".into();
                }
                ("target_arch", "x86") => return "(defined(__i386__) || defined(_M_IX86))".into(),
                ("target_arch", "aarch64") => {
                    return "(defined(__aarch64__) || defined(_M_ARM64))".into();
                }
                ("target_pointer_width", "64") => return "(UINTPTR_MAX == UINT64_MAX)".into(),
                ("target_pointer_width", "32") => return "(UINTPTR_MAX == UINT32_MAX)".into(),
                _ => {}
            }
            format!(
                "defined(FFIER_CFG_{}_{})",
                macro_part(key),
                macro_part(value)
            )
        }
        Cfg::All(items) | Cfg::Any(items) => {
            // `constant` already handled the decisive operands, so only
            // identity operands (`all()` inside `all`, `any()` inside `any`)
            // still need dropping to keep constants out of the guard.
            let all = matches!(cfg, Cfg::All(_));
            let mut parts: Vec<_> = items
                .iter()
                .filter(|item| constant(item) != Some(all))
                .map(expression)
                .collect();
            match parts.len() {
                1 => parts.pop().unwrap(),
                _ => format!("({})", parts.join(if all { " && " } else { " || " })),
            }
        }
        Cfg::Not(item) => format!("!({})", expression(item)),
    }
}

/// Emit `body` guarded by `cfg`.
///
/// Predicates that fold to a constant produce no preprocessor directives at
/// all: an always-enabled body is emitted as-is and an always-disabled body is
/// dropped. Only target-dependent predicates become `#if`/`#endif` pairs.
pub fn block(out: &mut String, cfg: Option<&Cfg>, body: impl FnOnce(&mut String)) {
    let Some(cfg) = cfg else {
        return body(out);
    };
    match constant(cfg) {
        Some(true) => body(out),
        Some(false) => {}
        None => {
            out.push_str(&format!("#if {}\n", expression(cfg)));
            body(out);
            out.push_str("#endif\n");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    #[test]
    fn constants_fold_out_of_guards() {
        let never = Cfg::Any(vec![]);
        let always = Cfg::All(vec![]);
        assert_eq!(constant(&never), Some(false));
        assert_eq!(constant(&always), Some(true));
        assert_eq!(constant(&Cfg::Not(Box::new(never.clone()))), Some(true));
        assert_eq!(
            constant(&Cfg::All(vec![always.clone(), never.clone()])),
            Some(false)
        );
        assert_eq!(constant(&Cfg::Any(vec![always, never])), Some(true));

        let unix = Cfg::Flag("unix".into());
        assert_eq!(constant(&unix), None);
        assert_eq!(
            constant(&Cfg::All(vec![Cfg::Not(Box::new(unix.clone()))])),
            None
        );

        // An unconditionally reserved vtable slot must not be guarded at all.
        let mut out = String::new();
        block(
            &mut out,
            Some(&Cfg::Not(Box::new(Cfg::Any(vec![])))),
            |out| out.push_str("reserved\n"),
        );
        assert_eq!(out, "reserved\n");

        // An always-disabled block is dropped entirely.
        out.clear();
        block(&mut out, Some(&Cfg::Any(vec![])), |out| {
            out.push_str("reserved\n")
        });
        assert_eq!(out, "");

        block(&mut out, Some(&unix), |out| out.push_str("body\n"));
        assert_eq!(
            out,
            "#if (defined(__unix__) || defined(__APPLE__))\nbody\n#endif\n"
        );
    }

    #[test]
    fn exact_os_guards_distinguish_android_and_apple_mobile_targets() {
        for (os, defines, enabled) in [
            ("linux", vec!["__linux__"], true),
            ("linux", vec!["__linux__", "__ANDROID__"], false),
            (
                "macos",
                vec!["__APPLE__", "__MACH__", "TARGET_OS_OSX=1"],
                true,
            ),
            (
                "macos",
                vec!["__APPLE__", "__MACH__", "TARGET_OS_OSX=0"],
                false,
            ),
        ] {
            let predicate = Cfg::Value {
                key: "target_os".into(),
                value: os.into(),
            };
            for (predicate, expected) in [
                (predicate.clone(), enabled),
                (Cfg::Not(Box::new(predicate)), !enabled),
            ] {
                let mut compiler = Command::new("cc");
                compiler.args(["-E", "-P", "-undef", "-x", "c", "-"]);
                for define in &defines {
                    compiler.arg(format!("-D{define}"));
                }
                let mut child = compiler
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap();
                writeln!(
                    child.stdin.take().unwrap(),
                    "#if {}\nenabled\n#else\ndisabled\n#endif",
                    expression(&predicate)
                )
                .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(output.status.success());
                assert_eq!(
                    String::from_utf8(output.stdout).unwrap().trim(),
                    if expected { "enabled" } else { "disabled" }
                );
            }
        }
    }
}
