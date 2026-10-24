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
                let predicate = match key.as_str() {
                    "all" => Cfg::All(map.next_value()?),
                    "any" => Cfg::Any(map.next_value()?),
                    "not" => Cfg::Not(map.next_value()?),
                    _ => {
                        if key.is_empty()
                            || !key.bytes().enumerate().all(|(i, b)| {
                                b == b'_'
                                    || b.is_ascii_alphabetic()
                                    || (i > 0 && b.is_ascii_digit())
                            })
                        {
                            return Err(M::Error::custom("invalid cfg predicate name"));
                        }
                        #[derive(Deserialize)]
                        #[serde(untagged)]
                        enum Leaf {
                            Flag(bool),
                            Value(String),
                        }
                        match map.next_value()? {
                            Leaf::Flag(true) => Cfg::Flag(key),
                            Leaf::Flag(false) => {
                                return Err(M::Error::custom("use not for negation"));
                            }
                            Leaf::Value(value) => Cfg::Value { key, value },
                        }
                    }
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
            Self::Flag(key) => f.write_str(key),
            Self::Value { key, value } => write!(f, "{key} = {value:?}"),
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
