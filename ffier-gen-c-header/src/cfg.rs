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

/// Translate target predicates using compiler predefined macros. Other cfg
/// names have an explicit FFIER_CFG_* override for the C consumer.
pub fn expression(cfg: &Cfg) -> String {
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
                ("target_os", "linux") => return "defined(__linux__)".into(),
                ("target_os", "macos") => {
                    return "(defined(__APPLE__) && defined(__MACH__))".into();
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
            let all = matches!(cfg, Cfg::All(_));
            if items.is_empty() {
                return if all { "1" } else { "0" }.into();
            }
            format!(
                "({})",
                items
                    .iter()
                    .map(expression)
                    .collect::<Vec<_>>()
                    .join(if all { " && " } else { " || " })
            )
        }
        Cfg::Not(item) => format!("!({})", expression(item)),
    }
}

pub fn start(out: &mut String, cfg: Option<&Cfg>) {
    if let Some(cfg) = cfg {
        out.push_str(&format!("#if {}\n", expression(cfg)));
    }
}

pub fn end(out: &mut String, cfg: Option<&Cfg>) {
    if cfg.is_some() {
        out.push_str("#endif\n");
    }
}
