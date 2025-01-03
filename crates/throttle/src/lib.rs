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
#[serde(try_from = "String")]
pub struct ThrottleSpec {
    pub limit: u64,
    /// Period, in seconds
    pub period: u64,
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
}

impl std::fmt::Debug for ThrottleSpec {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.as_string() {
            Ok(s) => write!(fmt, "{}", s),
            Err(_) => Err(std::fmt::Error),
        }
    }
}

impl std::fmt::Display for ThrottleSpec {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.as_string() {
            Ok(s) => write!(fmt, "{}", s),
            Err(_) => Err(std::fmt::Error),
        }
    }
}

impl ThrottleSpec {
    pub fn as_string(&self) -> Result<String, String> {
        let period = match self.period {
            86400 => "d",
            3600 => "h",
            60 => "m",
            1 => "s",
            _ => return Err(format!("cannot represent period {} as string", self.period)),
        };
        if let Some(burst) = self.max_burst {
            return Err(format!("cannot represent max_burst {burst} as string"));
        }

        Ok(format!(
            "{}{}/{period}",
            if self.force_local { "local:" } else { "" },
            self.limit
        ))
    }
}

impl TryFrom<String> for ThrottleSpec {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Self::try_from(s.as_str())
    }
}

impl TryFrom<&str> for ThrottleSpec {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, String> {
        let (force_local, s) = match s.strip_prefix("local:") {
            Some(s) => (true, s),
            None => (false, s),
        };
        let (limit, period) = s
            .split_once("/")
            .ok_or_else(|| format!("expected 'limit/period', got {s}"))?;

        let period = match period {
            "h" | "hr" | "hour" => 3600,
            "m" | "min" | "minute" => 60,
            "s" | "sec" | "second" => 1,
            "d" | "day" => 86400,
            invalid => return Err(format!("unknown period quantity {invalid}")),
        };

        // Allow "1_000/hr" and "1,000/hr" for more readable config
        let limit: String = limit
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
                "invalid ThrottleSpec `{s}`: limit must be greater than 0!"
            ));
        }

        Ok(Self {
            limit,
            period,
            max_burst: None,
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
            .as_string()
            .unwrap(),
            "100/h"
        );
        assert_eq!(
            ThrottleSpec {
                limit: 100,
                period: 3600,
                max_burst: None,
                force_local: true,
            }
            .as_string()
            .unwrap(),
            "local:100/h"
        );

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
