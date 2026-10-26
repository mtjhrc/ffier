use serde::{Deserialize, Serialize};
use std::fmt;

/// A conditional compilation predicate, independent of Rust token formatting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Cfg {
    Windows,
    Unix,
    TargetOs(String),
    Feature(String),
    All(Vec<Cfg>),
    Any(Vec<Cfg>),
    Not(Box<Cfg>),
}

/// Render Rust's `cfg` syntax when generating Rust source.
impl fmt::Display for Cfg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Windows => f.write_str("windows"),
            Self::Unix => f.write_str("unix"),
            Self::TargetOs(value) => write!(f, "target_os = {value:?}"),
            Self::Feature(value) => write!(f, "feature = {value:?}"),
            Self::All(items) | Self::Any(items) => {
                f.write_str(if matches!(self, Self::All(_)) {
                    "all("
                } else {
                    "any("
                })?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str(")")
            }
            Self::Not(item) => write!(f, "not({item})"),
        }
    }
}
