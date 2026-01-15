use config::{from_lua_value, get_or_create_sub_module};
use lru_cache::LruCache;
use lruttl::LruCacheWithTtl;
use mlua::{Lua, Value};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

static AUDIT_COUNTERS: LazyLock<Mutex<LruCacheWithTtl<String, Arc<AtomicUsize>>>> =
    LazyLock::new(|| Mutex::new(LruCacheWithTtl::new("audit_counters", 512)));

static AUDIT_DEFINITION: LazyLock<Mutex<LruCache<String, AuditConfig>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(256)));

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AuditConfig {
    #[serde(default)]
    pub bucket_count: usize,
    #[serde(default)]
    pub window: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AuditInput {
    pub name: String,
    pub key: String,
    pub count: usize,
}

fn duplicate_series_error(name: &str) -> mlua::Error {
    mlua::Error::RuntimeError(format!("Audit series '{name}' is already defined"))
}

fn missing_series_error(name: &str) -> mlua::Error {
    mlua::Error::RuntimeError(format!("Audit series '{name}' is not defined"))
}

fn current_epoch_secs() -> Result<u64, mlua::Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| mlua::Error::RuntimeError(format!("System time error: {err}")))
        .map(|duration| duration.as_secs())
}

async fn load_audit_config(name: &str) -> Result<AuditConfig, mlua::Error> {
    let mut cache = AUDIT_DEFINITION.lock().await;
    cache
        .get_mut(name)
        .map(|cfg| cfg.clone())
        .ok_or_else(|| missing_series_error(name))
}

pub async fn define_audit_series(name: &str, config: AuditConfig) -> Result<(), mlua::Error> {
    let mut cache = AUDIT_DEFINITION.lock().await;
    if cache.contains_key(name) {
        return Err(duplicate_series_error(name));
    }
    cache.insert(name.to_string(), config);
    Ok(())
}

pub async fn get_audit_series_total(name: &str) -> Result<usize, mlua::Error> {
    let now_secs = current_epoch_secs()?;
    let config = load_audit_config(name).await?;
    let current_bucket = (now_secs / config.window as u64) * config.window as u64;

    let counters = AUDIT_COUNTERS.lock().await;
    let mut total = 0usize;
    for offset in 0..config.bucket_count {
        let bucket_epoch = current_bucket.saturating_sub(offset as u64 * config.window as u64);
        let key = format!("{name}:{bucket_epoch}");
        if let Some(counter) = counters.get(&key) {
            total += counter.load(Ordering::Relaxed);
        }
    }

    Ok(total)
}

pub async fn add_audit_series_count(name: &str, count: usize) -> Result<usize, mlua::Error> {
    let now_secs = current_epoch_secs()?;
    let config = load_audit_config(name).await?;
    let bucket_epoch = (now_secs / config.window as u64) * config.window as u64;
    let key = format!("{name}:{bucket_epoch}");
    let ttl_secs = config.bucket_count as u64 * config.window as u64;

    let counters = AUDIT_COUNTERS.lock().await;
    if let Some(counter) = counters.get(&key) {
        let previous = counter.fetch_add(count, Ordering::SeqCst);
        return Ok(previous + count);
    }

    let counter = Arc::new(AtomicUsize::new(count));
    counters
        .insert(
            key,
            counter.clone(),
            Instant::now() + Duration::from_secs(ttl_secs),
        )
        .await;

    Ok(counter.load(Ordering::Relaxed))
}

pub async fn clear_audit_series(name: &str) -> Result<(), mlua::Error> {
    let now_secs = current_epoch_secs()?;
    let config = load_audit_config(name).await?;
    let current_bucket = (now_secs / config.window as u64) * config.window as u64;

    let counters = AUDIT_COUNTERS.lock().await;
    for offset in 0..config.bucket_count {
        let bucket_epoch = current_bucket.saturating_sub(offset as u64 * config.window as u64);
        let key = format!("{name}:{bucket_epoch}");
        if let Some(counter) = counters.get(&key) {
            counter.store(0, Ordering::Relaxed);
        }
    }

    Ok(())
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let audit = get_or_create_sub_module(lua, "audit_series")?;

    audit.set(
        "define",
        lua.create_async_function(|lua, (name, config): (String, Value)| async move {
            let config: AuditConfig = from_lua_value(&lua, config)?;
            define_audit_series(&name, config).await
        })?,
    )?;

    // Accumulate all buckets and return the total
    audit.set(
        "get",
        lua.create_async_function(
            |_, name: String| async move { get_audit_series_total(&name).await },
        )?,
    )?;

    // Add to the current bucket and return the existing value
    audit.set(
        "add",
        lua.create_async_function(|lua, (name, input): (String, Value)| async move {
            let input: AuditInput = from_lua_value(&lua, input)?;
            add_audit_series_count(&name, input.count).await
        })?,
    )?;

    // Reset the values of given audit name to 0 for all buckets
    audit.set(
        "reset",
        lua.create_async_function(
            |_, name: String| async move { clear_audit_series(&name).await },
        )?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_define_audit_series() {
        let name = "test_define";
        let config = AuditConfig {
            bucket_count: 5,
            window: 60,
        };

        let result = define_audit_series(name, config.clone()).await;
        assert!(result.is_ok());

        // Verify that defining the same series again returns an error
        let result = define_audit_series(name, config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already defined"));
    }

    #[tokio::test]
    async fn test_add_and_get_audit_series() {
        let name = "test_add_get";
        let config = AuditConfig {
            bucket_count: 3,
            window: 10,
        };

        define_audit_series(name, config).await.unwrap();

        // Add counts
        let result = add_audit_series_count(name, 5).await.unwrap();
        assert_eq!(result, 5);

        let result = add_audit_series_count(name, 3).await.unwrap();
        assert_eq!(result, 8);

        // Get total
        let total = get_audit_series_total(name).await.unwrap();
        assert_eq!(total, 8);
    }

    #[tokio::test]
    async fn test_clear_audit_series() {
        let name = "test_clear";
        let config = AuditConfig {
            bucket_count: 2,
            window: 5,
        };

        define_audit_series(name, config).await.unwrap();
        add_audit_series_count(name, 10).await.unwrap();

        let total = get_audit_series_total(name).await.unwrap();
        assert_eq!(total, 10);

        clear_audit_series(name).await.unwrap();

        let total = get_audit_series_total(name).await.unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn test_missing_series_error() {
        let result = get_audit_series_total("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not defined"));

        let result = add_audit_series_count("nonexistent", 1).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not defined"));
    }
}
