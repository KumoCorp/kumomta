use crate::{Error, ThrottleResult, REDIS};
use anyhow::Context;
use mod_redis::{Cmd, FromRedisValue, RedisConnection, Script};
use redis_cell_impl::{time, MemoryStore, Rate, RateLimiter, RateQuota};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static MEMORY: LazyLock<Mutex<MemoryStore>> = LazyLock::new(|| Mutex::new(MemoryStore::new()));

// Adapted from https://github.com/Losant/redis-gcra/blob/master/lib/gcra.lua
static GCRA_SCRIPT: LazyLock<Script> = LazyLock::new(|| {
    Script::new(
        r#"
local key = KEYS[1]
local limit = ARGV[1]
local period = ARGV[2]
local max_burst = ARGV[3]
local quantity = ARGV[4]

local interval = period / limit
local increment = interval * quantity
local burst_offset = interval * max_burst

local now = tonumber(redis.call("TIME")[1])
local tat = redis.call("GET", key)

if not tat then
  tat = now
else
  tat = tonumber(tat)
end
tat = math.max(tat, now)

local new_tat = tat + increment
local allow_at = new_tat - burst_offset
local diff = now - allow_at

local throttled
local reset_after
local retry_after

local remaining = math.floor(diff / interval) -- poor man's round

if remaining < 0 then
  throttled = 1
  -- calculate how many tokens there actually are, since
  -- remaining is how many there would have been if we had been able to limit
  -- and we did not limit
  remaining = math.floor((now - (tat - burst_offset)) / interval)
  reset_after = math.ceil(tat - now)
  retry_after = math.ceil(diff * -1)
elseif remaining == 0 and increment <= 0 then
  -- request with cost of 0
  -- cost of 0 with remaining 0 is still limited
  throttled = 1
  remaining = 0
  reset_after = math.ceil(tat - now)
  retry_after = 0 -- retry_after is meaningless when quantity is 0
else
  throttled = 0
  reset_after = math.ceil(new_tat - now)
  retry_after = 0
  redis.call("SET", key, new_tat, "EX", reset_after)
end

return {throttled, remaining, reset_after, retry_after, tostring(diff), tostring(interval)}
"#,
    )
});

fn local_throttle(
    key: &str,
    limit: u64,
    period: Duration,
    max_burst: u64,
    quantity: Option<u64>,
) -> Result<ThrottleResult, Error> {
    let mut store = MEMORY.lock().unwrap();
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
        retry_after: if retry_after <= 0 {
            None
        } else {
            Some(Duration::from_secs(retry_after.max(0) as u64))
        },
    })
}

async fn redis_cell_throttle(
    conn: &RedisConnection,
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
            n if n <= 0 => None,
            n => Some(Duration::from_secs(n as u64)),
        },
        reset_after: Duration::from_secs(result[4].max(0) as u64),
    })
}

async fn redis_script_throttle(
    conn: &RedisConnection,
    key: &str,
    limit: u64,
    period: Duration,
    max_burst: u64,
    quantity: Option<u64>,
) -> Result<ThrottleResult, Error> {
    let mut script = GCRA_SCRIPT.prepare_invoke();
    script
        .key(key)
        .arg(limit)
        .arg(period.as_secs())
        .arg(max_burst)
        .arg(quantity.unwrap_or(1));

    let result = conn
        .invoke_script(script)
        .await
        .context("error invoking redis GCRA script")?;
    let result =
        <(u64, u64, u64, u64, String, String) as FromRedisValue>::from_redis_value(&result)?;

    Ok(ThrottleResult {
        throttled: result.0 == 1,
        limit: max_burst + 1,
        remaining: result.1,
        retry_after: match result.3 {
            0 => None,
            n => Some(Duration::from_secs(n)),
        },
        reset_after: Duration::from_secs(result.2),
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
pub async fn throttle(
    key: &str,
    limit: u64,
    period: Duration,
    max_burst: u64,
    quantity: Option<u64>,
    force_local: bool,
) -> Result<ThrottleResult, Error> {
    match (force_local, REDIS.get()) {
        (false, Some(cx)) => {
            if cx.has_redis_cell {
                redis_cell_throttle(&cx, key, limit, period, max_burst, quantity).await
            } else {
                redis_script_throttle(&cx, key, limit, period, max_burst, quantity).await
            }
        }
        _ => local_throttle(key, limit, period, max_burst, quantity),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::redis::RedisContext;
    use mod_redis::test::RedisServer;

    trait Throttler {
        async fn throttle(
            &self,
            key: &str,
            limit: u64,
            period: Duration,
            max_burst: u64,
            quantity: Option<u64>,
        ) -> Result<ThrottleResult, Error>;
    }

    impl Throttler for Mutex<MemoryStore> {
        async fn throttle(
            &self,
            key: &str,
            limit: u64,
            period: Duration,
            max_burst: u64,
            quantity: Option<u64>,
        ) -> Result<ThrottleResult, Error> {
            local_throttle(key, limit, period, max_burst, quantity)
        }
    }

    struct RedisWithCell(RedisConnection);

    impl Throttler for RedisWithCell {
        async fn throttle(
            &self,
            key: &str,
            limit: u64,
            period: Duration,
            max_burst: u64,
            quantity: Option<u64>,
        ) -> Result<ThrottleResult, Error> {
            redis_cell_throttle(&self.0, key, limit, period, max_burst, quantity).await
        }
    }

    struct VanillaRedis(RedisConnection);

    impl Throttler for VanillaRedis {
        async fn throttle(
            &self,
            key: &str,
            limit: u64,
            period: Duration,
            max_burst: u64,
            quantity: Option<u64>,
        ) -> Result<ThrottleResult, Error> {
            redis_script_throttle(&self.0, key, limit, period, max_burst, quantity).await
        }
    }

    async fn test_big_limits(
        limit: u64,
        max_burst: Option<u64>,
        permitted_tolerance: f64,
        throttler: &impl Throttler,
    ) {
        let period = Duration::from_secs(60);
        let max_burst = max_burst.unwrap_or(limit);
        let key = format!("test_big_limits-{limit}-{max_burst}");

        let mut throttled_iter = None;

        for i in 0..limit * 2 {
            let result = throttler
                .throttle(&key, limit, period, max_burst, None)
                .await
                .unwrap();
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
            diff <= tolerance,
            "throttled after {throttled_iter} iterations for \
                limit {limit}. diff={diff} is not within tolerance {tolerance}"
        );
    }

    #[tokio::test]
    async fn basic_throttle_100() {
        test_big_limits(100, None, 0.01, &*MEMORY).await;
    }

    #[tokio::test]
    async fn basic_throttle_1_000() {
        test_big_limits(1_000, Some(100), 0.02, &*MEMORY).await;
    }

    #[tokio::test]
    async fn basic_throttle_6_000() {
        test_big_limits(6_000, Some(100), 0.02, &*MEMORY).await;
    }

    #[tokio::test]
    async fn basic_throttle_60_000() {
        test_big_limits(60_000, Some(100), 0.1, &*MEMORY).await;
    }

    #[tokio::test]
    async fn basic_throttle_60_000_burst_30k() {
        // Note that the 5% tolerance here is the same as the basic_throttle_60_000
        // test case because the variance is due to timing issues with very small
        // time periods produced by the overally limit, rather than the burst.
        test_big_limits(60_000, Some(100), 0.1, &*MEMORY).await;
    }

    #[tokio::test]
    async fn redis_cell_throttle_1_000() {
        if !RedisServer::is_available() {
            return;
        }

        let redis = RedisServer::spawn("").await.unwrap();
        let conn = redis.connection().await.unwrap();
        let cx = RedisContext::try_from(conn).await.unwrap();
        if !cx.has_redis_cell {
            return;
        }

        test_big_limits(1_000, None, 0.02, &RedisWithCell(cx.connection)).await;
    }

    #[tokio::test]
    async fn redis_script_throttle_1_000() {
        if !RedisServer::is_available() {
            return;
        }

        let redis = RedisServer::spawn("").await.unwrap();
        let conn = redis.connection().await.unwrap();
        let cx = RedisContext::try_from(conn).await.unwrap();
        test_big_limits(1_000, None, 0.2, &VanillaRedis(cx.connection)).await;
    }
}
