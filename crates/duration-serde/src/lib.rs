//! Based on <https://github.com/jean-airoldie/humantime-serde>
//! which is made available under the terms of the Apache 2.0 License.
//! This implementation allows for deserializing from integer and
//! floating point values; they are assumed to represent seconds.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::time::Duration;

/// A wrapper type which implements `Serialize` and `Deserialize` for
/// `Duration`
#[derive(Copy, Clone, Eq, Hash, PartialEq)]
pub struct Wrap<T>(T);

pub fn serialize<T, S>(d: &T, s: S) -> Result<S::Ok, S::Error>
where
    for<'a> Wrap<&'a T>: Serialize,
    S: Serializer,
{
    Wrap(d).serialize(s)
}

pub fn deserialize<'a, T, D>(d: D) -> Result<T, D::Error>
where
    Wrap<T>: Deserialize<'a>,
    D: Deserializer<'a>,
{
    Wrap::deserialize(d).map(|w| w.0)
}

impl<'de> Deserialize<'de> for Wrap<Duration> {
    fn deserialize<D>(d: D) -> Result<Wrap<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;

        impl<'de2> serde::de::Visitor<'de2> for V {
            type Value = Duration;

            fn expecting(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
                fmt.write_str("a duration")
            }

            fn visit_f64<E>(self, v: f64) -> Result<Duration, E>
            where
                E: serde::de::Error,
            {
                Ok(Duration::from_secs_f64(v))
            }

            fn visit_u64<E>(self, v: u64) -> Result<Duration, E>
            where
                E: serde::de::Error,
            {
                Ok(Duration::from_secs(v))
            }

            fn visit_i64<E>(self, v: i64) -> Result<Duration, E>
            where
                E: serde::de::Error,
            {
                match v.try_into() {
                    Ok(secs) => Ok(Duration::from_secs(secs)),
                    Err(err) => Err(E::custom(format!(
                        "duration must either be a string or a \
                         positive integer specifying the number of seconds. \
                         (error: {err:#})"
                    ))),
                }
            }

            fn visit_str<E>(self, v: &str) -> Result<Duration, E>
            where
                E: serde::de::Error,
            {
                humantime::parse_duration(v)
                    .map_err(|_| E::invalid_value(serde::de::Unexpected::Str(v), &self))
            }
        }

        d.deserialize_any(V).map(Wrap)
    }
}

impl<'de> Deserialize<'de> for Wrap<Option<Duration>> {
    fn deserialize<D>(d: D) -> Result<Wrap<Option<Duration>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Option::<Wrap<Duration>>::deserialize(d)? {
            Some(Wrap(dur)) => Ok(Wrap(Some(dur))),
            None => Ok(Wrap(None)),
        }
    }
}

impl<'a> Serialize for Wrap<&'a Duration> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        humantime::format_duration(*self.0)
            .to_string()
            .serialize(serializer)
    }
}

impl Serialize for Wrap<Duration> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        humantime::format_duration(self.0)
            .to_string()
            .serialize(serializer)
    }
}

impl<'a> Serialize for Wrap<&'a Option<Duration>> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self.0 {
            Some(dur) => serializer.serialize_some(&Wrap(dur)),
            None => serializer.serialize_none(),
        }
    }
}

impl Serialize for Wrap<Option<Duration>> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Wrap(&self.0).serialize(serializer)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn simple_string() {
        #[derive(Serialize, Deserialize)]
        struct Foo {
            #[serde(with = "super")]
            time: Duration,
        }

        let json = r#"{"time": "15 seconds"}"#;
        let foo = serde_json::from_str::<Foo>(json).unwrap();
        assert_eq!(foo.time, Duration::from_secs(15));
        let reverse = serde_json::to_string(&foo).unwrap();
        assert_eq!(reverse, r#"{"time":"15s"}"#);
    }

    #[test]
    fn simple_int() {
        #[derive(Serialize, Deserialize)]
        struct Foo {
            #[serde(with = "super")]
            time: Duration,
        }

        let json = r#"{"time": 15}"#;
        let foo = serde_json::from_str::<Foo>(json).unwrap();
        assert_eq!(foo.time, Duration::from_secs(15));
    }

    #[test]
    fn simple_float() {
        #[derive(Serialize, Deserialize)]
        struct Foo {
            #[serde(with = "super")]
            time: Duration,
        }

        let json = r#"{"time": 15.0}"#;
        let foo = serde_json::from_str::<Foo>(json).unwrap();
        assert_eq!(foo.time, Duration::from_secs(15));
    }
}
