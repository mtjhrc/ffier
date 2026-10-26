use std::collections::HashMap;

use ffier_schema::{Cfg, Library};

/// Return a predicate's truth value when it does not depend on its atoms.
pub fn constant(cfg: &Cfg) -> Option<bool> {
    match cfg {
        Cfg::Windows | Cfg::Unix | Cfg::TargetOs(_) | Cfg::Feature(_) => None,
        Cfg::All(items) | Cfg::Any(items) => {
            let all = matches!(cfg, Cfg::All(_));
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
    }
}

pub fn validate_library(lib: &Library) -> Result<(), String> {
    fn predicate(
        cfg: &Cfg,
        prefix: &str,
        guards: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        match cfg {
            Cfg::Feature(name) | Cfg::TargetOs(name) => {
                validate_name(name)?;
                let kind = match cfg {
                    Cfg::Feature(_) => "FEATURE",
                    Cfg::TargetOs(os) if os_guard(os).is_none() => "TARGET_OS",
                    _ => return Ok(()),
                };
                let guard = format!(
                    "{}_CFG_{kind}_{}",
                    prefix.to_ascii_uppercase(),
                    macro_part(name)
                );
                if let Some(previous) = guards.get(&guard) {
                    if previous != name {
                        return Err(format!(
                            "cfg names `{previous}` and `{name}` generate the same guard macro `{guard}`"
                        ));
                    }
                } else {
                    guards.insert(guard, name.clone());
                }
            }
            Cfg::All(items) | Cfg::Any(items) => {
                for item in items {
                    predicate(item, prefix, guards)?;
                }
            }
            Cfg::Not(item) => predicate(item, prefix, guards)?,
            Cfg::Windows | Cfg::Unix => {}
        }
        Ok(())
    }

    let methods = lib
        .exported_types
        .iter()
        .flat_map(|item| &item.methods)
        .chain(lib.traits.iter().flat_map(|item| &item.methods))
        .chain(lib.trait_impls.iter().flat_map(|item| &item.methods));
    let predicates = lib
        .exported_types
        .iter()
        .map(|item| item.cfg.as_ref())
        .chain(lib.traits.iter().map(|item| item.cfg.as_ref()))
        .chain(lib.trait_impls.iter().map(|item| item.cfg.as_ref()))
        .chain(lib.enum_constants.iter().map(|item| item.cfg.as_ref()))
        .chain(lib.bitflags_constants.iter().map(|item| item.cfg.as_ref()))
        .chain(lib.free_functions.iter().map(|item| item.cfg.as_ref()))
        .chain(methods.map(|method| method.cfg.as_ref()))
        .flatten();
    let mut guards = HashMap::new();
    for cfg in predicates {
        predicate(cfg, &lib.prefix, &mut guards)?;
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!(
            "unsupported cfg name `{name}`; use nonempty ASCII letters, digits, '_' or '-'"
        ));
    }
    Ok(())
}

fn os_guard(os: &str) -> Option<&'static str> {
    match os {
        "windows" => Some("defined(_WIN32)"),
        "linux" => Some("(defined(__linux__) && !defined(__ANDROID__))"),
        "macos" => Some("(defined(__APPLE__) && TARGET_OS_OSX)"),
        "freebsd" => Some("defined(__FreeBSD__)"),
        "openbsd" => Some("defined(__OpenBSD__)"),
        "netbsd" => Some("defined(__NetBSD__)"),
        "android" => Some("defined(__ANDROID__)"),
        _ => None,
    }
}

fn macro_part(value: &str) -> String {
    value.to_ascii_uppercase().replace('-', "_")
}

/// Translate platform predicates with compiler macros. Feature predicates and
/// OS values without a compiler mapping use library-prefixed guard macros.
pub fn expression(prefix: &str, cfg: &Cfg) -> String {
    if let Some(value) = constant(cfg) {
        return if value { "1" } else { "0" }.into();
    }
    match cfg {
        Cfg::Windows => "defined(_WIN32)".into(),
        Cfg::Unix => "(defined(__unix__) || defined(__APPLE__))".into(),
        Cfg::TargetOs(os) => os_guard(os).map_or_else(
            || {
                format!(
                    "defined({}_CFG_TARGET_OS_{})",
                    prefix.to_ascii_uppercase(),
                    macro_part(os)
                )
            },
            str::to_owned,
        ),
        Cfg::Feature(name) => format!(
            "defined({}_CFG_FEATURE_{})",
            prefix.to_ascii_uppercase(),
            macro_part(name)
        ),
        Cfg::All(items) | Cfg::Any(items) => {
            let all = matches!(cfg, Cfg::All(_));
            let mut parts: Vec<_> = items
                .iter()
                .filter(|item| constant(item) != Some(all))
                .map(|item| expression(prefix, item))
                .collect();
            match parts.len() {
                1 => parts.pop().unwrap(),
                _ => format!("({})", parts.join(if all { " && " } else { " || " })),
            }
        }
        Cfg::Not(item) => format!("!({})", expression(prefix, item)),
    }
}

/// Emit constant-true bodies directly, omit constant-false bodies, and guard
/// bodies whose predicates remain unresolved.
pub fn block(out: &mut String, prefix: &str, cfg: Option<&Cfg>, body: impl FnOnce(&mut String)) {
    let Some(cfg) = cfg else {
        return body(out);
    };
    match constant(cfg) {
        Some(true) => body(out),
        Some(false) => {}
        None => {
            out.push_str(&format!("#if {}\n", expression(prefix, cfg)));
            body(out);
            out.push_str("#endif\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_guards_distinguish_exact_os_values() {
        for (cfg, expected) in [
            (Cfg::Windows, "defined(_WIN32)"),
            (Cfg::Unix, "(defined(__unix__) || defined(__APPLE__))"),
            (
                Cfg::TargetOs("linux".into()),
                "(defined(__linux__) && !defined(__ANDROID__))",
            ),
            (
                Cfg::TargetOs("macos".into()),
                "(defined(__APPLE__) && TARGET_OS_OSX)",
            ),
            (
                Cfg::TargetOs("custom-os".into()),
                "defined(FT_CFG_TARGET_OS_CUSTOM_OS)",
            ),
        ] {
            assert_eq!(expression("ft", &cfg), expected);
        }

        #[cfg(unix)]
        {
            use std::{
                io::Write,
                process::{Command, Stdio},
            };

            for (os, defines, enabled) in [
                ("linux", vec!["__linux__"], true),
                ("linux", vec!["__linux__", "__ANDROID__"], false),
                ("macos", vec!["__APPLE__", "TARGET_OS_OSX=1"], true),
                ("macos", vec!["__APPLE__", "TARGET_OS_OSX=0"], false),
            ] {
                let cfg = Cfg::TargetOs(os.into());
                for (cfg, expected) in [(cfg.clone(), enabled), (Cfg::Not(Box::new(cfg)), !enabled)]
                {
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
                        expression("ft", &cfg)
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
}
