use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error, ser::SerializeMap};
use std::fmt;

/// A conditional compilation predicate, independent of Rust token formatting.
///
/// JSON examples: `{"unix": true}`, `{"target_os": "windows"}`,
/// `{"all": [{"unix": true}, {"feature": "files"}]}` and
/// `{"not": {"target_os": "windows"}}`. Each object has exactly one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cfg {
    Flag(String),
    Value { key: String, value: String },
    All(Vec<Cfg>),
    Any(Vec<Cfg>),
    Not(Box<Cfg>),
}

impl Serialize for Cfg {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Self::Flag(key) => map.serialize_entry(key, &true)?,
            Self::Value { key, value } => map.serialize_entry(key, value)?,
            Self::All(items) => map.serialize_entry("all", items)?,
            Self::Any(items) => map.serialize_entry("any", items)?,
            Self::Not(item) => map.serialize_entry("not", item)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Cfg {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Read entries directly so duplicate keys cannot silently overwrite
        // one another during deserialization.
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Cfg;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a cfg object with exactly one predicate")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Cfg, M::Error> {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| M::Error::custom("empty cfg"))?;
                if !key.chars().enumerate().all(|(i, c)| {
                    c == '_'
                        || if i == 0 {
                            unicode_ident::is_xid_start(c)
                        } else {
                            unicode_ident::is_xid_continue(c)
                        }
                }) || key.is_empty()
                {
                    return Err(M::Error::custom("invalid cfg predicate name"));
                }
                #[derive(Deserialize)]
                #[serde(untagged)]
                enum Value {
                    Flag(bool),
                    String(String),
                    Operands(Vec<Cfg>),
                    Operand(Box<Cfg>),
                }
                let predicate = match (key.as_str(), map.next_value::<Value>()?) {
                    // Operator names are also legal custom cfg names. The
                    // payload shape distinguishes an operator from a leaf.
                    ("all", Value::Operands(items)) => Cfg::All(items),
                    ("any", Value::Operands(items)) => Cfg::Any(items),
                    ("not", Value::Operand(item)) => Cfg::Not(item),
                    (_, Value::Flag(true)) => Cfg::Flag(key),
                    (_, Value::Flag(false)) => {
                        return Err(M::Error::custom("use not for negation"));
                    }
                    (_, Value::String(value)) => Cfg::Value { key, value },
                    _ => return Err(M::Error::custom("invalid cfg predicate payload")),
                };
                if map.next_key::<String>()?.is_some() {
                    return Err(M::Error::custom(
                        "cfg objects must contain exactly one predicate",
                    ));
                }
                Ok(predicate)
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

/// Render Rust's `cfg` syntax when generating Rust source.
impl fmt::Display for Cfg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flag(key) => write_key(f, key),
            Self::Value { key, value } => {
                write_key(f, key)?;
                write!(f, " = {value:?}")
            }
            Self::All(items) | Self::Any(items) => {
                f.write_str(if matches!(self, Self::All(_)) {
                    "all("
                } else {
                    "any("
                })?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
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

fn write_key(f: &mut fmt::Formatter<'_>, key: &str) -> fmt::Result {
    if matches!(
        key,
        "as" | "break"
            | "const"
            | "continue"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
            | "gen"
    ) {
        f.write_str("r#")?;
    }
    f.write_str(key)
}
