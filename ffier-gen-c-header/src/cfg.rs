use std::collections::HashSet;

use ffier_schema::Cfg;

/// Treatment of `feature = "..."` atoms not named in `--features`,
/// `--disable-features`, or `--guard-features`.
///
/// `Enabled` is the default because Cargo features are intended to be additive:
/// enabling a feature adds API without changing existing API. This describes
/// the largest API surface; extra declarations are harmless while unused.
///
/// Specify features that change an existing ABI explicitly. Use `--features`
/// or `--disable-features` to match the library build, making the header
/// specific to that build. Alternatively, use `--guard-features` so the
/// consumer selects the feature state that matches the library build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum UnspecifiedFeatures {
    /// Resolve unspecified feature atoms to true.
    #[default]
    Enabled,
    /// Resolve unspecified feature atoms to false.
    Disabled,
    /// Leave unspecified feature atoms unresolved for the consumer.
    Guard,
}

/// What the generator is allowed to assume about the build it describes.
///
/// Cargo feature atoms are resolved from the explicit enable/disable lists and
/// the fallback rule for everything else. Platform atoms stay unresolved unless
/// the caller asserts them with `--cfg`, which keeps a single header usable on
/// every target.
#[derive(Debug, Clone, Default)]
pub struct Assumptions {
    prefix: String,
    enabled_features: HashSet<String>,
    disabled_features: HashSet<String>,
    unspecified_features: UnspecifiedFeatures,
    /// Asserted cfg atoms, `(key, None)` for flags and `(key, Some(value))` for
    /// key/value predicates.
    asserted: HashSet<(String, Option<String>)>,
    /// Atoms the caller explicitly asked to keep guarded.
    guarded: HashSet<(String, Option<String>)>,
}

/// Keys that describe exactly one value per build. Asserting one of their
/// values makes the other values false.
const SINGLE_VALUED: [&str; 4] = [
    "target_os",
    "target_family",
    "target_arch",
    "target_pointer_width",
];

impl Assumptions {
    /// `prefix` namespaces the emitted guard macros, e.g. `ft` produces
    /// `FT_CFG_FEATURE_FILES`. Pass the library prefix, not the primitives
    /// prefix: guards describe this library's build.
    pub fn new(
        prefix: impl Into<String>,
        enabled_features: impl IntoIterator<Item = String>,
        disabled_features: impl IntoIterator<Item = String>,
        unspecified_features: UnspecifiedFeatures,
        asserted: impl IntoIterator<Item = (String, Option<String>)>,
        guarded: impl IntoIterator<Item = (String, Option<String>)>,
    ) -> Self {
        Self {
            prefix: prefix.into().to_ascii_uppercase(),
            enabled_features: enabled_features.into_iter().collect(),
            disabled_features: disabled_features.into_iter().collect(),
            unspecified_features,
            asserted: asserted.into_iter().collect(),
            guarded: guarded.into_iter().collect(),
        }
    }

    fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Parse a `KEY[=VALUE]` atom used by the `--cfg` and `--guard` flags.
    pub fn parse_atom(text: &str) -> Result<(String, Option<String>), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("expected KEY or KEY=VALUE".into());
        }
        match text.split_once('=') {
            Some((key, value)) => {
                let key = key.trim();
                let value = value.trim();
                if key.is_empty() {
                    return Err(format!("missing key in `{text}`"));
                }
                if value.is_empty() {
                    return Err(format!("missing value in `{text}`"));
                }
                Ok((key.to_string(), Some(value.to_string())))
            }
            None => Ok((text.to_string(), None)),
        }
    }

    fn feature(&self, name: &str) -> Option<bool> {
        if self
            .guarded
            .contains(&("feature".to_string(), Some(name.to_string())))
        {
            return None;
        }
        if self.enabled_features.contains(name) {
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

    /// Resolve one cfg atom, deriving the facts implied by an asserted value.
    fn atom(&self, key: &str, value: Option<&str>) -> Option<bool> {
        let atom = (key.to_string(), value.map(str::to_string));
        if self.guarded.contains(&atom) {
            return None;
        }
        if self.asserted.contains(&atom) {
            return Some(true);
        }
        match value {
            Some(value) => {
                // Another value of a single-valued key was asserted, so this
                // one cannot hold.
                if SINGLE_VALUED.contains(&key)
                    && self
                        .asserted
                        .iter()
                        .any(|(k, v)| k == key && v.as_deref() != Some(value))
                {
                    return Some(false);
                }
                // `target_family = "unix"` implies the `unix` flag and vice
                // versa. Handled by `flag` to keep the two in step.
                match (key, value) {
                    ("target_family", "unix") => self.flag("unix"),
                    ("target_family", "windows") => self.flag("windows"),
                    _ => None,
                }
            }
            None => self.flag(key),
        }
    }

    /// Resolve a bare flag such as `unix` or `windows`, deriving it from an
    /// asserted target description and from the mutually exclusive sibling.
    fn flag(&self, name: &str) -> Option<bool> {
        let atom = (name.to_string(), None);
        if self.guarded.contains(&atom) {
            return None;
        }
        if self.asserted.contains(&atom) {
            return Some(true);
        }
        let opposite = match name {
            "unix" => "windows",
            "windows" => "unix",
            _ => return None,
        };
        // The family pair is mutually exclusive, so asserting the sibling's
        // flag, family, or target rules this one out.
        let described = |wanted: &str| {
            self.asserted
                .iter()
                .any(|(key, value)| match (key.as_str(), value.as_deref()) {
                    ("target_family", Some(family)) => family == wanted,
                    ("target_os", Some(os)) => match wanted {
                        "windows" => os == "windows",
                        // Android runs a Linux userland but stays a distinct
                        // target; the header already separates it from desktop
                        // Linux.
                        _ => matches!(
                            os,
                            "linux" | "macos" | "freebsd" | "openbsd" | "netbsd" | "android"
                        ),
                    },
                    _ => false,
                })
        };
        if described(name) {
            Some(true)
        } else if described(opposite) || self.asserted.contains(&(opposite.to_string(), None)) {
            Some(false)
        } else {
            None
        }
    }

    /// Constant-fold a predicate under these assumptions. Returns `None` when
    /// the predicate still depends on how the consumer builds.
    pub fn constant(&self, cfg: &Cfg) -> Option<bool> {
        match cfg {
            Cfg::Flag(name) => self.atom(name, None),
            Cfg::Value { key, value } => match key.as_str() {
                "feature" => self.feature(value),
                _ => self.atom(key, Some(value)),
            },
            Cfg::All(items) | Cfg::Any(items) => {
                let all = matches!(cfg, Cfg::All(_));
                // A single dominating operand decides the combinator.
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

/// Translate target predicates using compiler predefined macros. Other cfg
/// names get a `<PREFIX>_CFG_*` override so that two headers with different
/// prefixes never collide in the consumer's macro namespace.
pub fn expression(assumptions: &Assumptions, cfg: &Cfg) -> String {
    if let Some(value) = assumptions.constant(cfg) {
        return if value { "1" } else { "0" }.into();
    }
    match cfg {
        Cfg::Flag(key) => match key.as_str() {
            "windows" => "defined(_WIN32)".into(),
            "unix" => "(defined(__unix__) || defined(__APPLE__))".into(),
            _ => format!("defined({}_CFG_{})", assumptions.prefix(), macro_part(key)),
        },
        Cfg::Value { key, value } => {
            match (key.as_str(), value.as_str()) {
                ("target_os" | "target_family", "windows") => return "defined(_WIN32)".into(),
                ("target_family", "unix") => {
                    return expression(assumptions, &Cfg::Flag("unix".into()));
                }
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
                "defined({}_CFG_{}_{})",
                assumptions.prefix(),
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

/// Emit `body` guarded by `cfg`.
///
/// Predicates that fold to a constant produce no preprocessor directives at
/// all: an always-enabled body is emitted as-is and an always-disabled body is
/// dropped. Only predicates that still depend on the consumer become
/// `#if`/`#endif` pairs.
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

    fn assume(asserted: &[(&str, Option<&str>)]) -> Assumptions {
        Assumptions::new(
            "ft",
            [],
            [],
            UnspecifiedFeatures::Guard,
            asserted
                .iter()
                .map(|(k, v)| (k.to_string(), v.map(str::to_string))),
            [],
        )
    }

    #[test]
    fn constants_fold_out_of_guards() {
        let assumptions = assume(&[]);
        let never = Cfg::Any(vec![]);
        let always = Cfg::All(vec![]);
        assert_eq!(assumptions.constant(&never), Some(false));
        assert_eq!(assumptions.constant(&always), Some(true));
        assert_eq!(
            assumptions.constant(&Cfg::Not(Box::new(never.clone()))),
            Some(true)
        );
        assert_eq!(
            assumptions.constant(&Cfg::All(vec![always.clone(), never.clone()])),
            Some(false)
        );
        assert_eq!(
            assumptions.constant(&Cfg::Any(vec![always, never])),
            Some(true)
        );

        let unix = Cfg::Flag("unix".into());
        assert_eq!(assumptions.constant(&unix), None);
        assert_eq!(
            assumptions.constant(&Cfg::All(vec![Cfg::Not(Box::new(unix.clone()))])),
            None
        );

        // An unconditionally reserved vtable slot must not be guarded at all.
        let mut out = String::new();
        block(
            &mut out,
            &assumptions,
            Some(&Cfg::Not(Box::new(Cfg::Any(vec![])))),
            |out| out.push_str("reserved\n"),
        );
        assert_eq!(out, "reserved\n");

        // An always-disabled block is dropped entirely.
        out.clear();
        block(&mut out, &assumptions, Some(&Cfg::Any(vec![])), |out| {
            out.push_str("reserved\n")
        });
        assert_eq!(out, "");

        block(&mut out, &assumptions, Some(&unix), |out| {
            out.push_str("body\n")
        });
        assert_eq!(
            out,
            "#if (defined(__unix__) || defined(__APPLE__))\nbody\n#endif\n"
        );
    }

    #[test]
    fn unspecified_features_follow_the_fallback_rule() {
        let feature = |name: &str| Cfg::Value {
            key: "feature".into(),
            value: name.into(),
        };
        for (mode, expected) in [
            (UnspecifiedFeatures::Enabled, Some(true)),
            (UnspecifiedFeatures::Disabled, Some(false)),
            (UnspecifiedFeatures::Guard, None),
        ] {
            let assumptions = Assumptions::new("ft", [], [], mode, [], []);
            assert_eq!(assumptions.constant(&feature("wide")), expected);
            assert_eq!(
                assumptions.constant(&Cfg::Not(Box::new(feature("wide")))),
                expected.map(|value| !value)
            );
        }

        // An explicit list wins over the fallback in both directions.
        let explicit = Assumptions::new(
            "ft",
            ["wide".to_string()],
            ["owner-api".to_string()],
            UnspecifiedFeatures::Guard,
            [],
            [],
        );
        assert_eq!(explicit.constant(&feature("wide")), Some(true));
        assert_eq!(explicit.constant(&feature("owner-api")), Some(false));
        assert_eq!(explicit.constant(&feature("unlisted")), None);
    }

    #[test]
    fn asserted_targets_derive_the_platform_flags() {
        let windows = assume(&[("target_os", Some("windows"))]);
        assert_eq!(windows.constant(&Cfg::Flag("windows".into())), Some(true));
        assert_eq!(windows.constant(&Cfg::Flag("unix".into())), Some(false));
        assert_eq!(
            windows.constant(&Cfg::Value {
                key: "target_os".into(),
                value: "linux".into()
            }),
            Some(false)
        );
        assert_eq!(
            windows.constant(&Cfg::Value {
                key: "target_os".into(),
                value: "windows".into()
            }),
            Some(true)
        );

        let linux = assume(&[("target_os", Some("linux"))]);
        assert_eq!(linux.constant(&Cfg::Flag("unix".into())), Some(true));
        assert_eq!(linux.constant(&Cfg::Flag("windows".into())), Some(false));
        // Exactly one OS, so the sibling values are ruled out.
        assert_eq!(
            linux.constant(&Cfg::Value {
                key: "target_os".into(),
                value: "macos".into()
            }),
            Some(false)
        );
        // An unrelated key stays open.
        assert_eq!(
            linux.constant(&Cfg::Value {
                key: "target_arch".into(),
                value: "aarch64".into()
            }),
            None
        );

        let flag_only = assume(&[("unix", None)]);
        assert_eq!(flag_only.constant(&Cfg::Flag("unix".into())), Some(true));
        assert_eq!(
            flag_only.constant(&Cfg::Flag("windows".into())),
            Some(false)
        );
        // A bare `unix` assertion does not say which OS.
        assert_eq!(
            flag_only.constant(&Cfg::Value {
                key: "target_os".into(),
                value: "linux".into()
            }),
            None
        );
    }

    #[test]
    fn explicit_guards_survive_a_conflicting_assumption() {
        let assumptions = Assumptions::new(
            "ft",
            ["wide".to_string()],
            [],
            UnspecifiedFeatures::Guard,
            [],
            [("feature".to_string(), Some("wide".to_string()))],
        );
        let feature = Cfg::Value {
            key: "feature".into(),
            value: "wide".into(),
        };
        assert_eq!(assumptions.constant(&feature), None);
        let mut out = String::new();
        block(&mut out, &assumptions, Some(&feature), |out| {
            out.push_str("body\n")
        });
        assert_eq!(out, "#if defined(FT_CFG_FEATURE_WIDE)\nbody\n#endif\n");
    }

    #[test]
    fn atom_parsing_rejects_empty_parts() {
        assert_eq!(
            Assumptions::parse_atom("target_os=windows"),
            Ok(("target_os".into(), Some("windows".into())))
        );
        assert_eq!(Assumptions::parse_atom(" unix "), Ok(("unix".into(), None)));
        assert!(Assumptions::parse_atom("").is_err());
        assert!(Assumptions::parse_atom("=windows").is_err());
        assert!(Assumptions::parse_atom("target_os=").is_err());
    }

    #[test]
    fn exact_os_guards_distinguish_android_and_apple_mobile_targets() {
        // Needs a C preprocessor to prove the emitted guard selects the same
        // target the predicate describes.
        #[cfg(unix)]
        {
            use std::{
                io::Write,
                process::{Command, Stdio},
            };

            let assumptions = assume(&[]);
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
                        expression(&assumptions, &predicate)
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
