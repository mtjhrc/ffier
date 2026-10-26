use std::collections::{HashMap, HashSet};

use crate::Options;
use ffier_schema::{Cfg, Library};

/// Treatment of Cargo features not listed in the explicit feature options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum UnspecifiedFeatures {
    /// Assume unlisted features are enabled.
    #[default]
    Enabled,
    /// Assume unlisted features are disabled.
    Disabled,
    /// Keep guards for unlisted features.
    Guard,
}

pub struct Assumptions {
    prefix: String,
    enabled_features: HashSet<String>,
    disabled_features: HashSet<String>,
    guarded_features: HashSet<String>,
    unspecified_features: UnspecifiedFeatures,
    asserted: Vec<Cfg>,
    guarded: Vec<Cfg>,
}

impl Assumptions {
    pub fn new(prefix: &str, options: &Options) -> Result<Self, String> {
        let platform_atoms = |atoms: &[String]| {
            atoms
                .iter()
                .map(|atom| parse_platform_atom(atom))
                .collect::<Result<Vec<_>, _>>()
        };
        let assumptions = Self {
            prefix: prefix.to_ascii_uppercase(),
            enabled_features: options.features.iter().cloned().collect(),
            disabled_features: options.disable_features.iter().cloned().collect(),
            guarded_features: options.guard_features.iter().cloned().collect(),
            unspecified_features: options.unspecified_features,
            asserted: platform_atoms(&options.cfg)?,
            guarded: platform_atoms(&options.guard)?,
        };
        let feature_lists = [
            ("--features", &assumptions.enabled_features),
            ("--disable-features", &assumptions.disabled_features),
            ("--guard-features", &assumptions.guarded_features),
        ];
        for (index, (name, features)) in feature_lists.iter().enumerate() {
            for feature in *features {
                validate_name(feature)?;
            }
            for (other_name, other_features) in &feature_lists[index + 1..] {
                if let Some(feature) = features.intersection(other_features).next() {
                    return Err(format!(
                        "feature `{feature}` appears in {name} and {other_name}"
                    ));
                }
            }
        }
        for (index, atom) in assumptions.asserted.iter().enumerate() {
            for other in &assumptions.asserted[index + 1..] {
                if contradict(atom, other) {
                    return Err(format!(
                        "contradictory --cfg assumptions: `{atom}` and `{other}`"
                    ));
                }
            }
        }
        Ok(assumptions)
    }

    fn feature(&self, name: &str) -> Option<bool> {
        if self.guarded_features.contains(name) {
            None
        } else if self.enabled_features.contains(name) {
            Some(true)
        } else if self.disabled_features.contains(name) {
            Some(false)
        } else {
            match self.unspecified_features {
                UnspecifiedFeatures::Enabled => Some(true),
                UnspecifiedFeatures::Disabled => Some(false),
                UnspecifiedFeatures::Guard => None,
            }
        }
    }

    fn platform(&self, atom: &Cfg) -> Option<bool> {
        if self.guarded.contains(atom) {
            return None;
        }
        if self.asserted.contains(atom) {
            return Some(true);
        }
        self.asserted.iter().find_map(|asserted| {
            if contradict(atom, asserted) {
                return Some(false);
            }
            match (atom, asserted) {
                (Cfg::Windows, Cfg::TargetOs(os)) => Some(os == "windows"),
                (Cfg::TargetOs(os), Cfg::Windows) => Some(os == "windows"),
                (Cfg::Unix, Cfg::TargetOs(os)) if is_unix_os(os) => Some(true),
                _ => None,
            }
        })
    }

    /// Resolve atoms from the caller's assumptions and fold Boolean constants.
    pub fn constant(&self, cfg: &Cfg) -> Option<bool> {
        match cfg {
            Cfg::Windows | Cfg::Unix | Cfg::TargetOs(_) => self.platform(cfg),
            Cfg::Feature(name) => self.feature(name),
            Cfg::All(items) | Cfg::Any(items) => {
                let all = matches!(cfg, Cfg::All(_));
                if items.iter().any(|item| self.constant(item) == Some(!all)) {
                    return Some(!all);
                }
                if items.iter().all(|item| self.constant(item).is_some()) {
                    return Some(all);
                }
                let complementary = items.iter().any(|item| {
                    let negated = Cfg::Not(Box::new(item.clone()));
                    items.contains(&negated)
                });
                complementary.then_some(!all)
            }
            Cfg::Not(item) => self.constant(item).map(|value| !value),
        }
    }
}

/// Parse only the supported platform atoms; feature options have separate flags.
fn parse_platform_atom(text: &str) -> Result<Cfg, String> {
    let text = text.trim();
    match text {
        "windows" => Ok(Cfg::Windows),
        "unix" => Ok(Cfg::Unix),
        _ => {
            let (_, value) = text
                .split_once('=')
                .filter(|(key, _)| key.trim() == "target_os")
                .ok_or_else(|| format!(
                    "unsupported platform predicate `{text}`; expected windows, unix, or target_os=VALUE"
                ))?;
            let value = value.trim();
            let value = if value.starts_with('"') {
                serde_json::from_str::<String>(value)
                    .map_err(|_| format!("invalid target_os value in `{text}`"))?
            } else {
                value.to_string()
            };
            validate_name(&value)?;
            Ok(Cfg::TargetOs(value))
        }
    }
}

fn is_unix_os(os: &str) -> bool {
    matches!(
        os,
        "linux" | "macos" | "freebsd" | "openbsd" | "netbsd" | "android"
    )
}

/// Facts whose simultaneous assertion would describe no supported target.
fn contradict(a: &Cfg, b: &Cfg) -> bool {
    match (a, b) {
        (Cfg::TargetOs(a), Cfg::TargetOs(b)) => a != b,
        (Cfg::Windows, Cfg::Unix) | (Cfg::Unix, Cfg::Windows) => true,
        (Cfg::Windows, Cfg::TargetOs(os)) | (Cfg::TargetOs(os), Cfg::Windows) => os != "windows",
        (Cfg::Unix, Cfg::TargetOs(os)) | (Cfg::TargetOs(os), Cfg::Unix) => os == "windows",
        _ => false,
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
        .chain(lib.value_structs.iter().map(|item| item.cfg.as_ref()))
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
pub fn expression(assumptions: &Assumptions, cfg: &Cfg) -> String {
    if let Some(value) = assumptions.constant(cfg) {
        return if value { "1" } else { "0" }.into();
    }
    match cfg {
        Cfg::Windows => "defined(_WIN32)".into(),
        Cfg::Unix => "(defined(__unix__) || defined(__APPLE__))".into(),
        Cfg::TargetOs(os) => os_guard(os).map_or_else(
            || {
                format!(
                    "defined({}_CFG_TARGET_OS_{})",
                    assumptions.prefix,
                    macro_part(os)
                )
            },
            str::to_owned,
        ),
        Cfg::Feature(name) => format!(
            "defined({}_CFG_FEATURE_{})",
            assumptions.prefix,
            macro_part(name)
        ),
        Cfg::All(items) | Cfg::Any(items) => {
            let all = matches!(cfg, Cfg::All(_));
            let mut parts: Vec<_> = items
                .iter()
                .filter(|item| assumptions.constant(item) != Some(all))
                .map(|item| expression(assumptions, item))
                .collect();
            match parts.len() {
                1 => parts.pop().unwrap(),
                _ => format!("({})", parts.join(if all { " && " } else { " || " })),
            }
        }
        Cfg::Not(item) => format!("!({})", expression(assumptions, item)),
    }
}

/// Emit constant-true bodies directly, omit constant-false bodies, and guard
/// bodies whose predicates remain unresolved.
pub fn block(
    out: &mut String,
    assumptions: &Assumptions,
    cfg: Option<&Cfg>,
    body: impl FnOnce(&mut String),
) {
    let Some(cfg) = cfg else {
        return body(out);
    };
    match assumptions.constant(cfg) {
        Some(true) => body(out),
        Some(false) => {}
        None => {
            out.push_str(&format!("#if {}\n", expression(assumptions, cfg)));
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
        let assumptions = Assumptions::new("ft", &Options::default()).unwrap();
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
            assert_eq!(expression(&assumptions, &cfg), expected);
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
                        expression(&assumptions, &cfg)
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
