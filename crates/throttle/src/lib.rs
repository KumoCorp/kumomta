//! This crate implements a throttling API based on a generic cell rate algorithm.
//! The implementation uses an in-memory store, but can be adjusted in the future
//! to support using a redis-cell equipped redis server to share the throttles
//! among multiple machines.
use once_cell::sync::OnceCell;
use redis_cell::cell::store::MemoryStore;
use redis_cell::cell::{Rate, RateLimiter, RateQuota};
use std::sync::Mutex;
use std::time::Duration;
use thiserror::Error;

static MEMORY: OnceCell<Mutex<MemoryStore>> = OnceCell::new();

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Generic(String),
}

#[derive(Debug, Eq, PartialEq)]
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
        time::Duration::from_std(period).map_err(|err| Error::Generic(format!("{err:#}")))?,
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
    let mut retry_after = rate_limit_result.retry_after.num_seconds();
    if rate_limit_result.retry_after.num_milliseconds() > 0 {
        retry_after += 1
    }
    let mut reset_after = rate_limit_result.reset_after.num_seconds();
    if rate_limit_result.reset_after.num_milliseconds() > 0 {
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
pub async fn throttle(
    key: &str,
    limit: u64,
    period: Duration,
    max_burst: u64,
    quantity: Option<u64>,
) -> Result<ThrottleResult, Error> {
    local_throttle(key, limit, period, max_burst, quantity)
}

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
        let max_rate = Rate::per_period(limit as i64, time::Duration::from_std(period).unwrap());
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
        test_big_limits(1_000, None, 0.01);
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
}
