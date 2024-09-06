//! This crate implements a throttling API based on a generic cell rate algorithm.
//! The implementation uses an in-memory store, but can be adjusted in the future
//! to support using a redis-cell equipped redis server to share the throttles
//! among multiple machines.
#[cfg(feature = "impl")]
use mod_redis::{Cmd, FromRedisValue, RedisConnection, RedisError};
#[cfg(feature = "impl")]
use once_cell::sync::OnceCell;
#[cfg(feature = "impl")]
use redis_cell_impl::{time, MemoryStore, Rate, RateLimiter, RateQuota};
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
#[cfg(feature = "impl")]
use std::sync::Mutex;
use std::time::Duration;
use thiserror::Error;

#[cfg(feature = "impl")]
pub mod limit;

#[cfg(feature = "impl")]
static MEMORY: OnceCell<Mutex<MemoryStore>> = OnceCell::new();
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
        throttle(
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
fn local_throttle(
    key: &str,
    limit: u64,
    period: Duration,
    max_burst: u64,
    quantity: Option<u64>,
) -> Result<ThrottleResult, Error> {
    let mut store = MEMORY
        .get_or_init(|| Mutex::new(MemoryStore::new()))
        .lock()
        .unwrap();
    let max_rate = Rate::per_period(
        limit as i64,
        time::Duration::try_from(period).map_err(|err| Error::Generic(format!("{err:#}")))?,
    );
    let mut limiter = RateLimiter::new(
        &mut *store,
        &RateQuota {
            max_burst: max_burst.min(limit - 1) as i64,
            max_rate,
        },
    );
    let quantity = quantity.unwrap_or(1) as i64;
    let (throttled, rate_limit_result) = limiter
        .rate_limit(key, quantity)
        .map_err(|err| Error::Generic(format!("{err:#}")))?;

    // If either time had a partial component, bump it up to the next full
    // second because otherwise a fast-paced caller could try again too
    // early.
    let mut retry_after = rate_limit_result.retry_after.whole_seconds();
    if rate_limit_result.retry_after.subsec_milliseconds() > 0 {
        retry_after += 1
    }
    let mut reset_after = rate_limit_result.reset_after.whole_seconds();
    if rate_limit_result.reset_after.subsec_milliseconds() > 0 {
        reset_after += 1
    }

    Ok(ThrottleResult {
        throttled,
        limit: rate_limit_result.limit as u64,
        remaining: rate_limit_result.remaining as u64,
        reset_after: Duration::from_secs(reset_after.max(0) as u64),
        retry_after: if retry_after == -1 {
            None
        } else {
            Some(Duration::from_secs(retry_after.max(0) as u64))
        },
    })
}

#[cfg(feature = "impl")]
async fn redis_throttle(
    conn: RedisConnection,
    key: &str,
    limit: u64,
    period: Duration,
    max_burst: u64,
    quantity: Option<u64>,
) -> Result<ThrottleResult, Error> {
    let mut cmd = Cmd::new();
    cmd.arg("CL.THROTTLE")
        .arg(key)
        .arg(max_burst)
        .arg(limit)
        .arg(period.as_secs())
        .arg(quantity.unwrap_or(1));
    let result = conn.query(cmd).await?;
    let result = <Vec<i64> as FromRedisValue>::from_redis_value(&result)?;

    Ok(ThrottleResult {
        throttled: result[0] != 0,
        limit: result[1] as u64,
        remaining: result[2] as u64,
        retry_after: match result[3] {
            n if n < 0 => None,
            n => Some(Duration::from_secs(n as u64)),
        },
        reset_after: Duration::from_secs(result[4].max(0) as u64),
    })
}

/// It is very important for `key` to be used with the same `limit`,
/// `period` and `max_burst` values in order to produce meaningful
/// results.
///
/// This interface cannot detect or report that kind of misuse.
/// It is recommended that those parameters be encoded into the
/// key to make it impossible to misuse.
///
/// * `limit` - specifies the maximum number of tokens allow
///             over the specified `period`
/// * `period` - the time period over which `limit` is allowed.
/// * `max_burst` - the maximum initial burst that will be permitted.
///                 set this smaller than `limit` to prevent using
///                 up the entire budget immediately and force it
///                 to spread out across time.
/// * `quantity` - how many tokens to add to the throttle. If omitted,
///                1 token is added.
/// * `force_local` - if true, always use the in-memory store on the local
///                   machine even if the redis backend has been configured.
#[cfg(feature = "impl")]
pub async fn throttle(
    key: &str,
    limit: u64,
    period: Duration,
    max_burst: u64,
    quantity: Option<u64>,
    force_local: bool,
) -> Result<ThrottleResult, Error> {
    if force_local {
        local_throttle(key, limit, period, max_burst, quantity)
    } else if let Some(redis) = REDIS.get().cloned() {
        redis_throttle(redis, key, limit, period, max_burst, quantity).await
    } else {
        local_throttle(key, limit, period, max_burst, quantity)
    }
}

#[cfg(feature = "impl")]
pub fn use_redis(conn: RedisConnection) -> Result<(), Error> {
    REDIS
        .set(conn)
        .map_err(|_| Error::Generic("redis already configured for throttles".to_string()))?;
    Ok(())
}

#[cfg(feature = "impl")]
#[cfg(test)]
mod test {
    use super::*;

    fn test_big_limits(limit: u64, max_burst: Option<u64>, permitted_tolerance: f64) {
        let period = Duration::from_secs(60);
        let max_burst = max_burst.unwrap_or(limit);
        let key = format!("test_big_limits-{limit}-{max_burst}");

        let mut throttled_iter = None;

        for i in 0..limit * 2 {
            let result = local_throttle(&key, limit, period, max_burst, None).unwrap();
            if result.throttled {
                println!("iter: {i} -> {result:?}");
                throttled_iter.replace(i);
                break;
            }
        }

        let throttled_iter = throttled_iter.expect("to hit the throttle limit");
        let diff = ((max_burst as f64) - (throttled_iter as f64)).abs();
        let tolerance = (max_burst as f64) * permitted_tolerance;
        println!(
            "throttled after {throttled_iter} iterations for \
                 limit {limit}. diff={diff}. tolerance {tolerance}"
        );
        let max_rate = Rate::per_period(limit as i64, time::Duration::try_from(period).unwrap());
        println!("max_rate: {max_rate:?}");

        assert!(
            diff < tolerance,
            "throttled after {throttled_iter} iterations for \
                limit {limit}. diff={diff} is not within tolerance {tolerance}"
        );
    }

    #[test]
    fn basic_throttle_100() {
        test_big_limits(100, None, 0.01);
    }

    #[test]
    fn basic_throttle_1_000() {
        test_big_limits(1_000, None, 0.02);
    }

    #[test]
    fn basic_throttle_6_000() {
        test_big_limits(6_000, None, 0.02);
    }

    #[test]
    fn basic_throttle_60_000() {
        test_big_limits(60_000, None, 0.05);
    }

    #[test]
    fn basic_throttle_60_000_burst_30k() {
        // Note that the 5% tolerance here is the same as the basic_throttle_60_000
        // test case because the variance is due to timing issues with very small
        // time periods produced by the overally limit, rather than the burst.
        test_big_limits(60_000, Some(30_000), 0.05);
    }

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
