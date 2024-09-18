//! This crate implements a throttling API based on a generic cell rate algorithm.
//! The implementation uses an in-memory store, but can be adjusted in the future
//! to support using a redis-cell equipped redis server to share the throttles
//! among multiple machines.
#[cfg(feature = "impl")]
use mod_redis::{RedisConnection, RedisError};
#[cfg(feature = "impl")]
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use std::time::Duration;
use thiserror::Error;

#[cfg(feature = "impl")]
pub mod limit;
#[cfg(feature = "impl")]
mod throttle;

#[cfg(feature = "impl")]
static REDIS: OnceCell<RedisConnection> = OnceCell::new();

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Generic(String),
    #[error("{0}")]
    AnyHow(#[from] anyhow::Error),
    #[cfg(feature = "impl")]
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

#[cfg(feature = "impl")]
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

#[cfg(feature = "impl")]
pub fn use_redis(conn: RedisConnection) -> Result<(), Error> {
    REDIS
        .set(conn)
        .map_err(|_| Error::Generic("redis already configured for throttles".to_string()))?;
    Ok(())
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
}
