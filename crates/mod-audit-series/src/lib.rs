use config::{SerdeWrappedValue, get_or_create_sub_module};
use lru_cache::LruCache;
use lruttl::LruCacheWithTtl;
use mlua::Lua;
use parking_lot::Mutex;
use serde::Deserialize;
use std::borrow::Borrow;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::sync::atomic::AtomicIsize;
use std::sync::{Arc, LazyLock};
use tokio::time::{Duration, Instant};

static AUDIT_COUNTERS: LazyLock<LruCacheWithTtl<AuditCounterKey, Arc<AtomicIsize>>> =
    LazyLock::new(|| LruCacheWithTtl::new("audit_counters", 256));

static AUDIT_DEFINITION: LazyLock<Mutex<LruCache<String, AuditConfig>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(64)));

static START: LazyLock<Instant> = LazyLock::new(Instant::now);

#[derive(Deserialize, Clone, Copy)]
pub struct AuditConfig {
    // The number of window the series will maintain
    pub window_count: usize,
    // Ttl of the created window
    #[serde(with = "duration_serde")]
    pub ttl: Duration,
}

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd, Debug)]
struct AuditCounterKey {
    /// series name, matches AUDIT_DEFINITION key
    series_name: String,
    /// The key to use within that series for the counter
    key: String,
    /// The ordinal of the window
    ordinal: u64,
}

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct BorrowedAuditCounterKey<'a> {
    series_name: &'a str,
    key: &'a str,
    ordinal: u64,
}

struct WindowInfo {
    /// The Ttl in seconds
    ttl_secs: u64,
    /// The ordinal of the window
    ordinal: u64,
}

trait Key {
    fn key<'k>(&'k self) -> BorrowedAuditCounterKey<'k>;
}

impl Key for AuditCounterKey {
    fn key<'k>(&'k self) -> BorrowedAuditCounterKey<'k> {
        BorrowedAuditCounterKey {
            series_name: self.series_name.as_str(),
            key: self.key.as_str(),
            ordinal: self.ordinal,
        }
    }
}

impl<'a> Key for BorrowedAuditCounterKey<'a> {
    fn key<'k>(&'k self) -> BorrowedAuditCounterKey<'k> {
        *self
    }
}

impl<'a> Borrow<dyn Key + 'a> for AuditCounterKey {
    fn borrow(&self) -> &(dyn Key + 'a) {
        self
    }
}

impl<'a> PartialEq for dyn Key + 'a {
    fn eq(&self, other: &Self) -> bool {
        self.key().eq(&other.key())
    }
}

impl<'a> Eq for dyn Key + 'a {}

impl<'a> PartialOrd for dyn Key + 'a {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.key().partial_cmp(&other.key())
    }
}

impl<'a> Ord for dyn Key + 'a {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

impl<'a> Hash for dyn Key + 'a {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state)
    }
}

fn elapsed_since_start_secs() -> u64 {
    START.elapsed().as_secs()
}

fn duplicate_series_error(name: &str) -> mlua::Error {
    mlua::Error::external(format!("Audit series '{name}' is already defined"))
}

fn missing_series_error(name: &str) -> mlua::Error {
    mlua::Error::external(format!("Audit series '{name}' is not defined"))
}

fn invalid_window_error() -> mlua::Error {
    mlua::Error::external("Audit series window must be at least one second")
}

fn current_window(ttl: Duration) -> WindowInfo {
    let ttl_secs = ttl.as_secs();
    let now_secs = elapsed_since_start_secs();
    let ordinal = (now_secs / ttl_secs) * ttl_secs;
    WindowInfo { ordinal, ttl_secs }
}

fn load_audit_config(name: &str) -> Result<AuditConfig, mlua::Error> {
    let mut cache = AUDIT_DEFINITION.lock();
    cache
        .get_mut(name)
        .map(|cfg| cfg.clone())
        .ok_or_else(|| missing_series_error(name))
}

pub fn define_audit_series(name: &str, config: AuditConfig) -> Result<(), mlua::Error> {
    if config.ttl == Duration::from_secs(0) {
        return Err(invalid_window_error());
    }
    let mut cache = AUDIT_DEFINITION.lock();
    if cache.contains_key(name) {
        return Err(duplicate_series_error(name));
    }
    cache.insert(name.to_string(), config);
    Ok(())
}

pub fn get_audit_series_total(name: &str, key: &str) -> Result<isize, mlua::Error> {
    let config = load_audit_config(name)?;
    let ttl = config.ttl;
    let window_info = current_window(ttl);

    let mut total = 0isize;
    for offset in 0..config.window_count {
        let window_start = window_info
            .ordinal
            .saturating_sub(offset as u64 * window_info.ttl_secs);
        let counter_key = BorrowedAuditCounterKey {
            series_name: name,
            key: key,
            ordinal: window_start,
        };
        if let Some(counter) = AUDIT_COUNTERS.get(&counter_key as &dyn Key) {
            total += counter.load(std::sync::atomic::Ordering::Relaxed);
        }
        // window_start being 0 means there's no older windows to check
        if window_start == 0 {
            break;
        }
    }

    Ok(total)
}

pub async fn add_audit_series_count(
    name: &str,
    key: &str,
    count: isize,
) -> Result<isize, mlua::Error> {
    let config = load_audit_config(name)?;
    let ttl = config.ttl;
    let window_info = current_window(ttl);
    let counter_key = AuditCounterKey {
        series_name: name.to_string(),
        key: key.to_string(),
        ordinal: window_info.ordinal,
    };
    let ttl_secs = config.window_count as u64 * window_info.ttl_secs;
    // Get existing counter or create a new one
    let counter = match AUDIT_COUNTERS.get(&counter_key) {
        Some(counter) => counter,
        None => {
            let expiration = tokio::time::Instant::now() + Duration::from_secs(ttl_secs);
            AUDIT_COUNTERS
                .insert(
                    counter_key.clone(),
                    Arc::new(AtomicIsize::new(0)),
                    expiration,
                )
                .await
        }
    };

    let previous = counter.fetch_add(count, std::sync::atomic::Ordering::SeqCst);
    Ok(previous + count)
}

pub fn reset_audit_series(name: &str, key: &str) -> Result<(), mlua::Error> {
    let config = load_audit_config(name)?;
    let ttl = config.ttl;
    let window_info = current_window(ttl);

    for offset in 0..config.window_count {
        let window_start = window_info
            .ordinal
            .saturating_sub(offset as u64 * window_info.ttl_secs);
        let counter_key = AuditCounterKey {
            series_name: name.to_string(),
            key: key.to_string(),
            ordinal: window_start,
        };
        if let Some(counter) = AUDIT_COUNTERS.get(&counter_key) {
            counter.store(0, std::sync::atomic::Ordering::Relaxed);
        }
    }

    Ok(())
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let audit = get_or_create_sub_module(lua, "audit_series")?;

    audit.set(
        "define",
        lua.create_function(
            |_lua, (name, config): (String, SerdeWrappedValue<AuditConfig>)| {
                let config = config.0;
                define_audit_series(&name, config)
            },
        )?,
    )?;

    audit.set(
        "get",
        lua.create_function(|_lua, (name, key): (String, String)| {
            get_audit_series_total(&name, &key)
        })?,
    )?;

    audit.set(
        "add",
        lua.create_async_function(
            |_lua, (name, key, count): (String, String, isize)| async move {
                add_audit_series_count(&name, &key, count).await
            },
        )?,
    )?;

    audit.set(
        "reset",
        lua.create_function(|_lua, (name, key): (String, String)| reset_audit_series(&name, &key))?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_define_audit_series() {
        let name = "test_define";
        let config = AuditConfig {
            window_count: 5,
            ttl: Duration::from_secs(60),
        };

        let result = define_audit_series(name, config.clone());
        assert!(result.is_ok());

        // Verify that defining the same series again returns an error
        let result = define_audit_series(name, config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already defined"));
    }

    #[tokio::test]
    async fn test_add_and_get_audit_series() {
        let name = "test_add_get";
        let config = AuditConfig {
            window_count: 3,
            ttl: Duration::from_secs(10),
        };

        define_audit_series(name, config).unwrap();
        let key = "entity".to_string();

        // Add counts
        let result = add_audit_series_count(name, &key, 5).await.unwrap();
        assert_eq!(result, 5);

        let result = add_audit_series_count(name, &key, 3).await.unwrap();
        assert_eq!(result, 8);

        // Get total
        let total = get_audit_series_total(name, &key).unwrap();
        assert_eq!(total, 8);
    }

    #[tokio::test]
    async fn test_reset_audit_series() {
        let name = "test_reset";
        let config = AuditConfig {
            window_count: 2,
            ttl: Duration::from_secs(5),
        };

        define_audit_series(name, config).unwrap();
        let key = "entity_reset".to_string();
        add_audit_series_count(name, &key, 10).await.unwrap();

        let total = get_audit_series_total(name, &key).unwrap();
        assert_eq!(total, 10);

        reset_audit_series(name, &key).unwrap();

        let total = get_audit_series_total(name, &key).unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn test_missing_series_error() {
        let result = get_audit_series_total("nonexistent", "missing");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not defined"));

        let result = add_audit_series_count("nonexistent", "missing", 1).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not defined"));
    }

    #[tokio::test]
    async fn test_invalid_ttl() {
        let name = "test_invalid_ttl";
        let config = AuditConfig {
            window_count: 3,
            ttl: Duration::from_secs(0),
        };

        let result = define_audit_series(name, config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("at least one second")
        );
    }

    #[tokio::test]
    async fn test_negative_counts() {
        let name = "test_negative";
        let config = AuditConfig {
            window_count: 2,
            ttl: Duration::from_secs(10),
        };

        define_audit_series(name, config).unwrap();
        let key = "negative_entity";

        add_audit_series_count(name, key, 100).await.unwrap();
        let result = add_audit_series_count(name, key, -30).await.unwrap();
        assert_eq!(result, 70);

        let total = get_audit_series_total(name, key).unwrap();
        assert_eq!(total, 70);
    }

    #[tokio::test]
    async fn test_different_keys_same_series() {
        let name = "test_different_keys";
        let config = AuditConfig {
            window_count: 2,
            ttl: Duration::from_secs(10),
        };

        define_audit_series(name, config).unwrap();

        add_audit_series_count(name, "key1", 5).await.unwrap();
        add_audit_series_count(name, "key2", 10).await.unwrap();

        let total1 = get_audit_series_total(name, "key1").unwrap();
        let total2 = get_audit_series_total(name, "key2").unwrap();

        assert_eq!(total1, 5);
        assert_eq!(total2, 10);
    }
}
