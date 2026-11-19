use kumo_redact::REDACTED;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A configured value, or its display-only replacement in an inspection copy.
/// An outer `Option` distinguishes either of these from an unset field.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ConfigSecret<T> {
    Value(T),
    Withheld,
}

impl<T> ConfigSecret<T> {
    pub fn expose(&self) -> anyhow::Result<&T> {
        match self {
            Self::Value(value) => Ok(value),
            Self::Withheld => anyhow::bail!("cannot use a withheld configuration value"),
        }
    }
}

impl<T> fmt::Debug for ConfigSecret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&REDACTED, f)
    }
}

impl<T: Serialize> Serialize for ConfigSecret<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(value) => value.serialize(serializer),
            Self::Withheld => serializer.serialize_str(REDACTED),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for ConfigSecret<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Treat the display marker as ordinary policy input. For example, a
        // password or filename can literally be "[REDACTED]".
        T::deserialize(deserializer).map(Self::Value)
    }
}
