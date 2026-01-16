use config::{from_lua_value, get_or_create_sub_module};
use lru_cache::LruCache;
use lruttl::LruCacheWithTtl;
use mlua::{Lua, Value};
use serde::Deserialize;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

static AUDIT_COUNTERS: LazyLock<Mutex<LruCacheWithTtl<String, Arc<AtomicIsize>>>> =
    LazyLock::new(|| Mutex::new(LruCacheWithTtl::new("audit_counters", 512)));

static AUDIT_DEFINITION: LazyLock<Mutex<LruCache<String, AuditConfig>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(256)));

#[derive(Deserialize, Clone)]
pub struct AuditConfig {
    pub bucket_count: usize,
    #[serde(with = "duration_serde")]
    pub window: Option<Duration>,
}

#[derive(Deserialize)]
pub struct AuditInput {
    pub key: String,
    #[serde(default)]
    pub count: isize,
}

fn duplicate_series_error(name: &str) -> mlua::Error {
    mlua::Error::external(format!("Audit series '{name}' is already defined"))
}

fn missing_series_error(name: &str) -> mlua::Error {
    mlua::Error::external(format!("Audit series '{name}' is not defined"))
}

fn current_epoch_secs() -> Result<u64, mlua::Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| mlua::Error::external(format!("System time error: {err}")))
        .map(|duration| duration.as_secs())
}

fn window_secs(window: Duration) -> Result<u64, mlua::Error> {
    let secs = window.as_secs();
    if secs == 0 {
        return Err(mlua::Error::external(
            "Audit series window must be at least one second",
        ));
    }
    Ok(secs)
}

fn current_bucket(window: Duration) -> Result<(u64, u64), mlua::Error> {
    let window_secs = window_secs(window)?;
    let now_secs = current_epoch_secs()?;
    Ok(((now_secs / window_secs) * window_secs, window_secs))
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

pub async fn get_audit_series_total(name: &str, input: AuditInput) -> Result<isize, mlua::Error> {
    let config = load_audit_config(name).await?;
    let window = config.window.ok_or_else(|| {
        mlua::Error::external(format!("Audit series '{name}' window is not configured"))
    })?;
    let (bucket_epoch, window_secs) = current_bucket(window)?;

    let counters = AUDIT_COUNTERS.lock().await;
    let mut total = 0isize;
    for offset in 0..config.bucket_count {
        let bucket_start = bucket_epoch.saturating_sub(offset as u64 * window_secs);
        let key = format!("{}:{}", input.key, bucket_start);
        if let Some(counter) = counters.get(&key) {
            total += counter.load(Ordering::Relaxed);
        }
    }

    Ok(total)
}

pub async fn add_audit_series_count(name: &str, input: AuditInput) -> Result<isize, mlua::Error> {
    let config = load_audit_config(name).await?;
    let window = config.window.ok_or_else(|| {
        mlua::Error::external(format!("Audit series '{name}' window is not configured"))
    })?;
    let (bucket_epoch, window_secs) = current_bucket(window)?;
    let key = format!("{}:{}", input.key, bucket_epoch);
    // Additional time
    let ttl_secs = config.bucket_count as u64 * window_secs;

    let counters = AUDIT_COUNTERS.lock().await;
    if let Some(counter) = counters.get(&key) {
        let previous = counter.fetch_add(input.count, Ordering::SeqCst);
        return Ok(previous + input.count);
    }

    let counter = Arc::new(AtomicIsize::new(input.count));
    counters
        .insert(
            key,
            counter.clone(),
            Instant::now() + Duration::from_secs(ttl_secs),
        )
        .await;

    Ok(counter.load(Ordering::Relaxed))
}

pub async fn reset_audit_series(name: &str, input: AuditInput) -> Result<(), mlua::Error> {
    let config = load_audit_config(name).await?;
    let window = config.window.ok_or_else(|| {
        mlua::Error::external(format!("Audit series '{name}' window is not configured"))
    })?;
    let (bucket_epoch, window_secs) = current_bucket(window)?;

    let counters = AUDIT_COUNTERS.lock().await;
    for offset in 0..config.bucket_count {
        let bucket_start = bucket_epoch.saturating_sub(offset as u64 * window_secs);
        let key = format!("{}:{}", input.key, bucket_start);
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
        lua.create_async_function(|lua, (name, input): (String, Value)| async move {
            let input: AuditInput = from_lua_value(&lua, input)?;
            get_audit_series_total(&name, input).await
        })?,
    )?;

    // Add to the current bucket and return the existing value
    audit.set(
        "add",
        lua.create_async_function(|lua, (name, input): (String, Value)| async move {
            let input: AuditInput = from_lua_value(&lua, input)?;
            add_audit_series_count(&name, input).await
        })?,
    )?;

    // Reset the values of given audit name to 0 for all buckets
    audit.set(
        "reset",
        lua.create_async_function(|lua, (name, input): (String, Value)| async move {
            let input: AuditInput = from_lua_value(&lua, input)?;
            reset_audit_series(&name, input).await
        })?,
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
            bucket_count: 5,
            window: Some(Duration::from_secs(60)),
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
            window: Some(Duration::from_secs(10)),
        };

        define_audit_series(name, config).await.unwrap();
        let key = "entity".to_string();

        // Add counts
        let result = add_audit_series_count(
            name,
            AuditInput {
                key: key.clone(),
                count: 5,
            },
        )
        .await
        .unwrap();
        assert_eq!(result, 5);

        let result = add_audit_series_count(
            name,
            AuditInput {
                key: key.clone(),
                count: 3,
            },
        )
        .await
        .unwrap();
        assert_eq!(result, 8);

        // Get total
        let total = get_audit_series_total(
            name,
            AuditInput {
                key: key.clone(),
                count: 0,
            },
        )
        .await
        .unwrap();
        assert_eq!(total, 8);
    }

    #[tokio::test]
    async fn test_reset_audit_series() {
        let name = "test_reset";
        let config = AuditConfig {
            bucket_count: 2,
            window: Some(Duration::from_secs(5)),
        };

        define_audit_series(name, config).await.unwrap();
        let key = "entity_reset".to_string();
        add_audit_series_count(
            name,
            AuditInput {
                key: key.clone(),
                count: 10,
            },
        )
        .await
        .unwrap();

        let total = get_audit_series_total(
            name,
            AuditInput {
                key: key.clone(),
                count: 0,
            },
        )
        .await
        .unwrap();
        assert_eq!(total, 10);

        reset_audit_series(
            name,
            AuditInput {
                key: key.clone(),
                count: 0,
            },
        )
        .await
        .unwrap();

        let total = get_audit_series_total(name, AuditInput { key, count: 0 })
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn test_missing_series_error() {
        let result = get_audit_series_total(
            "nonexistent",
            AuditInput {
                key: "missing".to_string(),
                count: 0,
            },
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not defined"));

        let result = add_audit_series_count(
            "nonexistent",
            AuditInput {
                key: "missing".to_string(),
                count: 1,
            },
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not defined"));
    }
}
