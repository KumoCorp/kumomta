//! Reports the field names a `Deserialize` impl accepts, without
//! deserializing any real data.
use serde::de::{self, Deserialize, Deserializer, Visitor};
use serde::forward_to_deserialize_any;
use std::fmt;

/// The outcome of probing a type's `Deserialize` impl.
#[derive(Debug)]
enum Probe {
    /// The field names the impl declared to `deserialize_struct`.
    Fields(&'static [&'static str]),
    /// The impl asked for something other than a struct.
    NotAStruct,
    /// The impl raised its own error before asking for anything.
    Custom(String),
}

impl fmt::Display for Probe {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Fields(fields) => write!(fmt, "struct with fields {fields:?}"),
            Self::NotAStruct => write!(fmt, "not a struct"),
            Self::Custom(msg) => write!(fmt, "{msg}"),
        }
    }
}

impl std::error::Error for Probe {}

impl de::Error for Probe {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }
}

/// A `Deserializer` that drives a type's `derive(Deserialize)` impl just far
/// enough to learn its field names, without ever producing a value.
/// `deserialize_struct` returns the field list it receives, as an error.
/// Every other entry point returns `NotAStruct`.
struct FieldNameProbe;

impl<'de> Deserializer<'de> for FieldNameProbe {
    type Error = Probe;

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Probe> {
        Err(Probe::Fields(fields))
    }

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Probe> {
        Err(Probe::NotAStruct)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

/// Returns the field names of `T` as they appear in input data, after `rename`
/// and `rename_all`, for a plain `derive(Deserialize)` struct. Returns `None`
/// for a struct with `#[serde(flatten)]`, an enum, a newtype, or a hand-written
/// impl.
///
/// A field with `#[serde(alias)]` appears only under its primary name. Serde's
/// derive does not include aliases in that name list. Callers that classify
/// input keys against this list will treat an alias as unknown.
pub fn struct_field_names<T: for<'de> Deserialize<'de>>() -> Option<&'static [&'static str]> {
    match T::deserialize(FieldNameProbe) {
        Err(Probe::Fields(fields)) => Some(fields),
        Ok(_) | Err(Probe::NotAStruct) | Err(Probe::Custom(_)) => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct Plain {
        alpha: u32,
        #[serde(rename = "beta_renamed")]
        beta: String,
        #[serde(default)]
        gamma: Option<bool>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "kebab-case")]
    #[allow(dead_code)]
    struct Kebab {
        first_field: u32,
        second_field: u32,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Inner {
        x: u32,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Flattened {
        a: u32,
        #[serde(flatten)]
        inner: Inner,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    enum Kind {
        One,
        Two,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Newtype(u32);

    #[test]
    fn plain_struct_reports_renamed_fields() {
        k9::assert_equal!(
            struct_field_names::<Plain>(),
            Some(&["alpha", "beta_renamed", "gamma"][..])
        );
    }

    #[test]
    fn rename_all_is_applied() {
        k9::assert_equal!(
            struct_field_names::<Kebab>(),
            Some(&["first-field", "second-field"][..])
        );
    }

    #[test]
    fn flattened_struct_has_no_field_list() {
        k9::assert_equal!(struct_field_names::<Flattened>(), None);
    }

    #[test]
    fn non_structs_have_no_field_list() {
        k9::assert_equal!(struct_field_names::<Kind>(), None);
        k9::assert_equal!(struct_field_names::<Newtype>(), None);
        k9::assert_equal!(struct_field_names::<u32>(), None);
    }
}
