//! This crate implements a throttling API based on a generic cell rate algorithm.
//! The implementation uses an in-memory store, but can be adjusted in the future
//! to support using a redis-cell equipped redis server to share the throttles
//! among multiple machines.
#[cfg(feature = "redis")]
use mod_redis::RedisError;
use serde::{Deserialize, Deserializer, Serialize};
use std::convert::TryFrom;
use std::time::Duration;
use thiserror::Error;

#[cfg(feature = "redis")]
pub mod limit;
#[cfg(feature = "redis")]
mod throttle;

#[cfg(feature = "redis")]
mod redis {
    use super::*;
    use mod_redis::{Cmd, RedisConnection, RedisValue};
    use std::ops::Deref;
    use std::sync::OnceLock;

    #[derive(Debug)]
    pub(crate) struct RedisContext {
        pub(crate) connection: RedisConnection,
        pub(crate) has_redis_cell: bool,
    }

    impl RedisContext {
        pub async fn try_from(connection: RedisConnection) -> anyhow::Result<Self> {
            let mut cmd = Cmd::new();
            cmd.arg("COMMAND").arg("INFO").arg("CL.THROTTLE");

            let rsp = connection.query(cmd).await?;
            let has_redis_cell = rsp
                .as_sequence()
                .map_or(false, |arr| arr.iter().any(|v| v != &RedisValue::Nil));

            Ok(Self {
                has_redis_cell,
                connection,
            })
        }
    }

    impl Deref for RedisContext {
        type Target = RedisConnection;
        fn deref(&self) -> &Self::Target {
            &self.connection
        }
    }

    pub(crate) static REDIS: OnceLock<RedisContext> = OnceLock::new();

    pub async fn use_redis(conn: RedisConnection) -> Result<(), Error> {
        REDIS
            .set(RedisContext::try_from(conn).await?)
            .map_err(|_| Error::Generic("redis already configured for throttles".to_string()))?;
        Ok(())
    }
}

#[cfg(feature = "redis")]
pub use redis::use_redis;
#[cfg(feature = "redis")]
pub(crate) use redis::REDIS;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Generic(String),
    #[error("{0}")]
    AnyHow(#[from] anyhow::Error),
    #[cfg(feature = "redis")]
    #[error("{0}")]
    Redis(#[from] RedisError),
    #[error("TooManyLeases, try again in {0:?}")]
    TooManyLeases(Duration),
    #[error("NonExistentLease")]
    NonExistentLease,
}

#[derive(Eq, PartialEq, Clone, Copy, Serialize, Deserialize, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct ThrottleSpec {
    pub limit: u64,
    /// Period, in seconds
    pub period: u64,
    /// Constrain how quickly the throttle can be consumed per
    /// throttle interval (period / limit).
    /// max_burst defaults to limit, allowing the entire throttle
    /// to be used in an instant. Setting max_burst to 1 will
    /// only allow 1 throttle bump per interval, spreading out
    /// the utilization of the throttle more evenly over time.
    /// Larger values allow more of the throttle to be used
    /// per period.
    pub max_burst: Option<u64>,
    pub force_local: bool,
}

#[cfg(feature = "redis")]
impl ThrottleSpec {
    pub async fn throttle<S: AsRef<str>>(&self, key: S) -> Result<ThrottleResult, Error> {
        self.throttle_quantity(key, 1).await
    }

    pub async fn throttle_quantity<S: AsRef<str>>(
        &self,
        key: S,
        quantity: u64,
    ) -> Result<ThrottleResult, Error> {
        let key = key.as_ref();
        let limit = self.limit;
        let period = self.period;
        let max_burst = self.max_burst.unwrap_or(limit);
        let key = format!("{key}:{limit}:{max_burst}:{period}");
        throttle::throttle(
            &key,
            limit,
            Duration::from_secs(period),
            max_burst,
            Some(quantity),
            self.force_local,
        )
        .await
    }

    /// Returns the effective burst value for this throttle spec
    pub fn burst(&self) -> u64 {
        self.max_burst.unwrap_or(self.limit)
    }

    /// Returns the throttle interval over which the burst applies
    pub fn interval(&self) -> Duration {
        Duration::from_secs_f64(self.period as f64 / self.limit.max(1) as f64)
    }
}

impl std::fmt::Debug for ThrottleSpec {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}", self.as_string())
    }
}

impl std::fmt::Display for ThrottleSpec {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}", self.as_string())
    }
}

impl ThrottleSpec {
    pub fn as_string(&self) -> String {
        let mut period_scale = None;
        let period = match self.period {
            86400 => "d",
            3600 => "h",
            60 => "m",
            1 => "s",
            other => {
                period_scale.replace(other.to_string());
                "s"
            }
        };

        let burst = match self.max_burst {
            Some(b) => format!(",max_burst={b}"),
            None => String::new(),
        };

        format!(
            "{}{}/{}{period}{burst}",
            if self.force_local { "local:" } else { "" },
            self.limit,
            match &period_scale {
                Some(scale) => scale.as_str(),
                None => "",
            }
        )
    }
}

impl From<ThrottleSpec> for String {
    fn from(spec: ThrottleSpec) -> String {
        spec.as_string()
    }
}

impl TryFrom<String> for ThrottleSpec {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Self::try_from(s.as_str())
    }
}

fn opt_digit_prefix(s: &str) -> Result<(u64, &str), String> {
    let mut n: Option<u64> = None;
    for (idx, c) in s.char_indices() {
        if !c.is_ascii_digit() {
            return Ok((n.unwrap_or(1), &s[idx..]));
        }

        let byte = c as u8;

        let digit = match byte.checked_sub(b'0') {
            None => return Err(format!("invalid digit {c}")),
            Some(digit) if digit > 9 => return Err(format!("invalid digit {c}")),
            Some(digit) => {
                debug_assert!((0..=9).contains(&digit));
                u64::from(digit)
            }
        };

        n = Some(
            n.take()
                .unwrap_or(0)
                .checked_mul(10)
                .and_then(|n| n.checked_add(digit))
                .ok_or_else(|| format!("number too big"))?,
        );
    }

    Err(format!("invalid period quantity {s}"))
}

/// Allow "1_000" and "1,000" for more readable config
fn parse_separated_number(limit: &str) -> Result<u64, String> {
    let value: String = limit
        .chars()
        .filter_map(|c| match c {
            '_' | ',' => None,
            c => Some(c),
        })
        .collect();

    value
        .parse::<u64>()
        .map_err(|err| format!("invalid limit '{limit}': {err:#}"))
}

impl TryFrom<&str> for ThrottleSpec {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        let (force_local, s) = match s.strip_prefix("local:") {
            Some(s) => (true, s),
            None => (false, s),
        };

        let (s, max_burst) = match s.split_once(",max_burst=") {
            Some((s, burst_spec)) => {
                let burst = parse_separated_number(burst_spec)?;
                (s, Some(burst))
            }
            None => (s, None),
        };

        let (limit, period) = s
            .split_once("/")
            .ok_or_else(|| format!("expected 'limit/period', got {s}"))?;

        let (period_scale, period) = opt_digit_prefix(period)?;

        let period = match period {
            "h" | "hr" | "hour" => 3600,
            "m" | "min" | "minute" => 60,
            "s" | "sec" | "second" => 1,
            "d" | "day" => 86400,
            invalid => return Err(format!("unknown period quantity {invalid}")),
        } * period_scale;

        // Allow "1_000/hr" and "1,000/hr" for more readable config
        let limit = parse_separated_number(limit)?;

        if limit == 0 {
            return Err(format!(
                "invalid ThrottleSpec `{s}`: limit must be greater than 0!"
            ));
        }

        Ok(Self {
            limit,
            period,
            max_burst,
            force_local,
        })
    }
}

#[derive(Debug, Eq, PartialEq, Serialize)]
pub struct ThrottleResult {
    /// true if the action was limited
    pub throttled: bool,
    /// The total limit of the key (max_burst + 1). This is equivalent to the common
    /// X-RateLimit-Limit HTTP header.
    pub limit: u64,
    /// The remaining limit of the key. Equivalent to X-RateLimit-Remaining.
    pub remaining: u64,
    /// The number of seconds until the limit will reset to its maximum capacity.
    /// Equivalent to X-RateLimit-Reset.
    pub reset_after: Duration,
    /// The number of seconds until the user should retry, but None if the action was
    /// allowed. Equivalent to Retry-After.
    pub retry_after: Option<Duration>,
}

#[derive(Eq, PartialEq, Clone, Copy, Serialize, Hash)]
pub struct LimitSpec {
    /// Maximum amount
    pub limit: u64,
    pub force_local: bool,
}

impl std::fmt::Debug for LimitSpec {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.force_local {
            write!(fmt, "local:{:?}", self.limit)
        } else {
            self.limit.fmt(fmt)
        }
    }
}

impl std::fmt::Display for LimitSpec {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{:?}", self)
    }
}

impl LimitSpec {
    pub const fn new(limit: u64) -> Self {
        Self {
            limit,
            force_local: false,
        }
    }
}

impl TryFrom<&str> for LimitSpec {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        let (force_local, s) = match s.strip_prefix("local:") {
            Some(s) => (true, s),
            None => (false, s),
        };

        // Allow "1_000/hr" and "1,000/hr" for more readable config
        let limit: String = s
            .chars()
            .filter_map(|c| match c {
                '_' | ',' => None,
                c => Some(c),
            })
            .collect();

        let limit = limit
            .parse::<u64>()
            .map_err(|err| format!("invalid limit '{limit}': {err:#}"))?;

        if limit == 0 {
            return Err(format!(
                "invalid LimitSpec `{s}`: limit must be greater than 0!"
            ));
        }

        Ok(Self { limit, force_local })
    }
}

impl<'de> Deserialize<'de> for LimitSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Visitor;

        struct Helper {}
        impl<'de> Visitor<'de> for Helper {
            type Value = LimitSpec;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("string or numeric limit spec")
            }

            fn visit_str<E>(self, value: &str) -> Result<LimitSpec, E>
            where
                E: serde::de::Error,
            {
                value.try_into().map_err(|err| E::custom(err))
            }

            fn visit_i8<E>(self, value: i8) -> Result<LimitSpec, E>
            where
                E: serde::de::Error,
            {
                if value < 1 {
                    return Err(E::custom("limit must be 1 or larger"));
                }
                Ok(LimitSpec {
                    limit: value as u64,
                    force_local: false,
                })
            }

            fn visit_i16<E>(self, value: i16) -> Result<LimitSpec, E>
            where
                E: serde::de::Error,
            {
                if value < 1 {
                    return Err(E::custom("limit must be 1 or larger"));
                }
                Ok(LimitSpec {
                    limit: value as u64,
                    force_local: false,
                })
            }

            fn visit_i32<E>(self, value: i32) -> Result<LimitSpec, E>
            where
                E: serde::de::Error,
            {
                if value < 1 {
                    return Err(E::custom("limit must be 1 or larger"));
                }
                Ok(LimitSpec {
                    limit: value as u64,
                    force_local: false,
                })
            }

            fn visit_i64<E>(self, value: i64) -> Result<LimitSpec, E>
            where
                E: serde::de::Error,
            {
                if value < 1 {
                    return Err(E::custom("limit must be 1 or larger"));
                }
                Ok(LimitSpec {
                    limit: value as u64,
                    force_local: false,
                })
            }

            fn visit_u8<E>(self, value: u8) -> Result<LimitSpec, E>
            where
                E: serde::de::Error,
            {
                if value < 1 {
                    return Err(E::custom("limit must be 1 or larger"));
                }
                Ok(LimitSpec {
                    limit: value as u64,
                    force_local: false,
                })
            }

            fn visit_u16<E>(self, value: u16) -> Result<LimitSpec, E>
            where
                E: serde::de::Error,
            {
                if value < 1 {
                    return Err(E::custom("limit must be 1 or larger"));
                }
                Ok(LimitSpec {
                    limit: value as u64,
                    force_local: false,
                })
            }

            fn visit_u32<E>(self, value: u32) -> Result<LimitSpec, E>
            where
                E: serde::de::Error,
            {
                if value < 1 {
                    return Err(E::custom("limit must be 1 or larger"));
                }
                Ok(LimitSpec {
                    limit: value as u64,
                    force_local: false,
                })
            }

            fn visit_u64<E>(self, value: u64) -> Result<LimitSpec, E>
            where
                E: serde::de::Error,
            {
                if value < 1 {
                    return Err(E::custom("limit must be 1 or larger"));
                }
                Ok(LimitSpec {
                    limit: value as u64,
                    force_local: false,
                })
            }
        }

        deserializer.deserialize_any(Helper {})
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn throttle_spec_parse() {
        assert_eq!(
            ThrottleSpec::try_from("100/hr").unwrap(),
            ThrottleSpec {
                limit: 100,
                period: 3600,
                max_burst: None,
                force_local: false,
            }
        );
        assert_eq!(
            ThrottleSpec::try_from("local:100/hr").unwrap(),
            ThrottleSpec {
                limit: 100,
                period: 3600,
                max_burst: None,
                force_local: true,
            }
        );

        assert_eq!(
            ThrottleSpec {
                limit: 100,
                period: 3600,
                max_burst: None,
                force_local: false,
            }
            .as_string(),
            "100/h"
        );
        assert_eq!(
            ThrottleSpec {
                limit: 100,
                period: 3600,
                max_burst: None,
                force_local: true,
            }
            .as_string(),
            "local:100/h"
        );

        let weird_duration = ThrottleSpec::try_from("local:100/123m").unwrap();
        assert_eq!(
            weird_duration,
            ThrottleSpec {
                limit: 100,
                period: 123 * 60,
                max_burst: None,
                force_local: true,
            }
        );
        assert_eq!(weird_duration.as_string(), "local:100/7380s");

        assert_eq!(
            ThrottleSpec::try_from("1_0,0/hour").unwrap(),
            ThrottleSpec {
                limit: 100,
                period: 3600,
                max_burst: None,
                force_local: false,
            }
        );
        assert_eq!(
            ThrottleSpec::try_from("100/our").unwrap_err(),
            "unknown period quantity our".to_string()
        );
        assert_eq!(
            ThrottleSpec::try_from("three/hour").unwrap_err(),
            "invalid limit 'three': invalid digit found in string".to_string()
        );

        let burst = ThrottleSpec::try_from("50/day,max_burst=1").unwrap();
        assert_eq!(
            burst,
            ThrottleSpec {
                limit: 50,
                period: 86400,
                max_burst: Some(1),
                force_local: false,
            }
        );
        assert_eq!(burst.as_string(), "50/d,max_burst=1");
        assert_eq!(burst.burst(), 1);
        assert_eq!(format!("{:?}", burst.interval()), "1728s");
    }

    #[test]
    fn test_opt_digit_prefix() {
        assert_eq!(opt_digit_prefix("m").unwrap(), (1, "m"));
        assert_eq!(
            opt_digit_prefix("1").unwrap_err(),
            "invalid period quantity 1"
        );
        assert_eq!(opt_digit_prefix("1q").unwrap(), (1, "q"));
        assert_eq!(opt_digit_prefix("2s").unwrap(), (2, "s"));
        assert_eq!(opt_digit_prefix("20s").unwrap(), (20, "s"));
        assert_eq!(opt_digit_prefix("12378s").unwrap(), (12378, "s"));
    }

    #[test]
    fn limit_spec_parse() {
        assert_eq!(LimitSpec::try_from("100").unwrap(), LimitSpec::new(100));
        assert_eq!(LimitSpec::try_from("1_00").unwrap(), LimitSpec::new(100));
        assert_eq!(
            LimitSpec::try_from("local:1_00").unwrap(),
            LimitSpec {
                limit: 100,
                force_local: true
            }
        );
        assert_eq!(
            LimitSpec::try_from("three").unwrap_err(),
            "invalid limit 'three': invalid digit found in string".to_string()
        );
    }
}
