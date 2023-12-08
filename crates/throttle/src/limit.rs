use crate::{Error, REDIS};
use anyhow::anyhow;
use mod_redis::{RedisConnection, Script};
use once_cell::sync::{Lazy, OnceCell};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

static MEMORY: OnceCell<Mutex<MemoryStore>> = OnceCell::new();

static ACQUIRE_SCRIPT: Lazy<Script> = Lazy::new(|| {
    Script::new(
        r#"
local now_ts = tonumber(ARGV[1])
local expires_ts = tonumber(ARGV[2])
local limit = tonumber(ARGV[3])
local uuid = ARGV[4]
local tomorrow_ts = now_ts + 86400

-- prune expired values
redis.call("ZREMRANGEBYSCORE", KEYS[1], 0, now_ts-1)

local count = redis.call("ZCOUNT", KEYS[1], now_ts, tomorrow_ts)
if count + 1 > limit then
  -- find the next expiration time
  local smallest = redis.call("ZRANGE", KEYS[1], "-inf", "+inf", "BYSCORE", "LIMIT", 0, 1, "WITHSCORES")
  -- smallest holds the uuid and its expiration time;
  -- we want to just return the remaining time interval
  return smallest[2] - now_ts
end
redis.call("ZADD", KEYS[1], "NX", expires_ts, uuid)
return redis.status_reply('OK')
"#,
    )
});

pub struct LimitSpec {
    /// Maximum amount
    pub limit: usize,
    /// Maximum lease duration for a single count
    pub duration: Duration,
}

#[derive(Debug)]
pub struct LimitLease {
    /// Name of the element to release on Drop
    name: String,
    uuid: Uuid,
    armed: bool,
    backend: Backend,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Backend {
    Memory,
    Redis,
}

impl LimitSpec {
    pub async fn acquire_lease<S: AsRef<str>>(&self, key: S) -> Result<LimitLease, Error> {
        if let Some(redis) = REDIS.get().cloned() {
            self.acquire_lease_redis(redis, key.as_ref()).await
        } else {
            self.acquire_lease_memory(key.as_ref()).await
        }
    }

    pub async fn acquire_lease_redis(
        &self,
        conn: RedisConnection,
        key: &str,
    ) -> Result<LimitLease, Error> {
        let now_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let expires_ts = now_ts + self.duration.as_secs();
        let uuid = Uuid::new_v4();
        let uuid_str = uuid.to_string();

        let mut script = ACQUIRE_SCRIPT.prepare_invoke();
        script
            .key(key)
            .arg(now_ts)
            .arg(expires_ts)
            .arg(self.limit)
            .arg(uuid_str);

        match conn.invoke_script(script).await? {
            mod_redis::RedisValue::Okay => {}
            mod_redis::RedisValue::Int(next_expiration_interval) => {
                return Err(Error::TooManyLeases(Duration::from_secs(
                    next_expiration_interval as u64,
                )));
            }
            value => {
                return Err(anyhow!("acquire script succeeded but returned {value:?}").into());
            }
        }

        Ok(LimitLease {
            name: key.to_string(),
            uuid,
            armed: true,
            backend: Backend::Redis,
        })
    }

    pub async fn acquire_lease_memory(&self, key: &str) -> Result<LimitLease, Error> {
        let uuid = Uuid::new_v4();
        let mut store = MEMORY
            .get_or_init(|| Mutex::new(MemoryStore::new()))
            .lock()
            .unwrap();

        let set = store.get_or_create(key);
        set.expire_old();

        set.acquire(uuid, self.limit, self.duration)?;

        Ok(LimitLease {
            name: key.to_string(),
            uuid,
            armed: true,
            backend: Backend::Memory,
        })
    }
}

impl LimitLease {
    pub async fn release(&mut self) {
        self.armed = false;
        match self.backend {
            Backend::Memory => self.release_memory().await,
            Backend::Redis => {
                if let Some(redis) = REDIS.get().cloned() {
                    self.release_redis(redis).await;
                } else {
                    eprintln!("LimitLease::release: backend is Redis but REDIS is not set");
                }
            }
        }
    }

    pub async fn extend(&self, duration: Duration) -> Result<(), Error> {
        match self.backend {
            Backend::Memory => self.extend_memory(duration).await,
            Backend::Redis => {
                if let Some(redis) = REDIS.get().cloned() {
                    self.extend_redis(redis, duration).await
                } else {
                    Err(anyhow::anyhow!(
                        "LimitLease::extend: backend is Redis but REDIS is not set"
                    )
                    .into())
                }
            }
        }
    }

    pub fn take(&mut self) -> Self {
        let armed = self.armed;
        self.armed = false;
        Self {
            name: self.name.clone(),
            uuid: self.uuid,
            armed,
            backend: self.backend,
        }
    }

    async fn extend_memory(&self, duration: Duration) -> Result<(), Error> {
        let mut store = MEMORY
            .get()
            .ok_or_else(|| anyhow!("MEMORY is not initialized"))?
            .lock()
            .unwrap();
        if let Some(set) = store.get(&self.name) {
            set.extend(self.uuid, duration)
        } else {
            Err(Error::NonExistentLease)
        }
    }

    async fn extend_redis(&self, conn: RedisConnection, duration: Duration) -> Result<(), Error> {
        let now_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let expires = now_ts + duration.as_secs();

        let mut cmd = mod_redis::cmd("ZADD");
        cmd.arg(&self.name)
            .arg("XX") // only allow updating existing
            .arg("CH") // return number of changed entries
            .arg(expires)
            .arg(self.uuid.to_string());
        let value = conn.query(cmd).await?;

        if value != mod_redis::RedisValue::Int(1) {
            return Err(anyhow!("Failed to extend lease").into());
        }

        Ok(())
    }

    async fn release_memory(&mut self) {
        if let Some(store) = MEMORY.get() {
            let mut store = store.lock().unwrap();
            if let Some(set) = store.get(&self.name) {
                set.release(self.uuid);
            }
        }
    }

    async fn release_redis(&mut self, conn: RedisConnection) {
        let mut cmd = mod_redis::cmd("ZREM");
        cmd.arg(&self.name).arg(self.uuid.to_string());
        conn.query(cmd).await.ok();
    }
}

impl Drop for LimitLease {
    fn drop(&mut self) {
        if self.armed {
            self.armed = false;
            let mut deferred = Self {
                armed: false,
                name: self.name.clone(),
                uuid: self.uuid,
                backend: self.backend,
            };
            tokio::task::Builder::new()
                .name("LimitLeaseDropper")
                .spawn(async move {
                    deferred.release().await;
                })
                .ok();
        }
    }
}

struct LeaseSet {
    members: HashMap<Uuid, Instant>,
}

impl LeaseSet {
    fn new() -> Self {
        Self {
            members: HashMap::new(),
        }
    }

    fn expire_old(&mut self) {
        let now = Instant::now();
        self.members.retain(|_k, expiry| *expiry > now);
    }

    fn acquire(&mut self, uuid: Uuid, limit: usize, duration: Duration) -> Result<(), Error> {
        if self.members.len() + 1 > limit {
            let min_expiration = self.members.values().min().expect("some elements");
            Err(Error::TooManyLeases(*min_expiration - Instant::now()))
        } else {
            self.members.insert(uuid, Instant::now() + duration);
            Ok(())
        }
    }

    fn extend(&mut self, uuid: Uuid, duration: Duration) -> Result<(), Error> {
        match self.members.get_mut(&uuid) {
            Some(entry) => {
                *entry = Instant::now() + duration;
                Ok(())
            }
            None => Err(Error::NonExistentLease),
        }
    }

    fn release(&mut self, uuid: Uuid) {
        self.members.remove(&uuid);
    }
}

struct MemoryStore {
    sets: HashMap<String, LeaseSet>,
}

impl MemoryStore {
    fn new() -> Self {
        Self {
            sets: HashMap::new(),
        }
    }

    fn get(&mut self, name: &str) -> Option<&mut LeaseSet> {
        self.sets.get_mut(name)
    }

    fn get_or_create(&mut self, name: &str) -> &mut LeaseSet {
        self.sets
            .entry(name.to_string())
            .or_insert_with(|| LeaseSet::new())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use mod_redis::test::RedisServer;

    #[tokio::test]
    async fn test_memory() {
        let limit = LimitSpec {
            limit: 2,
            duration: Duration::from_secs(2),
        };

        let key = format!("test_memory-{}", Uuid::new_v4());
        let lease1 = limit.acquire_lease_memory(&key).await.unwrap();
        eprintln!("lease1: {lease1:?}");
        let mut lease2 = limit.acquire_lease_memory(&key).await.unwrap();
        eprintln!("lease2: {lease2:?}");
        // Cannot acquire a 3rd lease while the other two are alive
        assert!(limit.acquire_lease_memory(&key).await.is_err());

        // Release and try to get a third
        lease2.release().await;
        let _lease3 = limit.acquire_lease_memory(&key).await.unwrap();

        // Cannot acquire while the other two are alive
        assert!(limit.acquire_lease_memory(&key).await.is_err());

        // Wait for some number of leases to expire
        tokio::time::sleep(limit.duration + limit.duration).await;

        // We can acquire another now
        let _lease4 = limit.acquire_lease_memory(&key).await.unwrap();
    }

    #[tokio::test]
    async fn test_redis() {
        if which::which("redis-server").is_err() {
            return;
        }
        let redis = RedisServer::spawn().await.unwrap();
        let conn = redis.connection().await.unwrap();

        let limit = LimitSpec {
            limit: 2,
            duration: Duration::from_secs(2),
        };

        let key = format!("test_redis-{}", Uuid::new_v4());
        let mut lease1 = limit.acquire_lease_redis(conn.clone(), &key).await.unwrap();
        eprintln!("lease1: {lease1:?}");
        let mut lease2 = limit.acquire_lease_redis(conn.clone(), &key).await.unwrap();
        eprintln!("lease2: {lease2:?}");
        // Cannot acquire a 3rd lease while the other two are alive
        assert!(limit.acquire_lease_redis(conn.clone(), &key).await.is_err());

        // Release and try to get a third
        lease2.release_redis(conn.clone()).await;
        let mut lease3 = limit.acquire_lease_redis(conn.clone(), &key).await.unwrap();

        // Cannot acquire while the other two are alive
        assert!(limit.acquire_lease_redis(conn.clone(), &key).await.is_err());

        // Wait for some number of leases to expire
        tokio::time::sleep(limit.duration + limit.duration).await;

        let mut lease4 = limit.acquire_lease_redis(conn.clone(), &key).await.unwrap();

        lease1.release_redis(conn.clone()).await;
        lease3.release_redis(conn.clone()).await;
        lease4.release_redis(conn.clone()).await;
    }

    #[tokio::test]
    async fn test_memory_extension() {
        let limit = LimitSpec {
            limit: 1,
            duration: Duration::from_secs(2),
        };

        let key = format!("test_redis-{}", Uuid::new_v4());
        let lease1 = limit.acquire_lease_memory(&key).await.unwrap();
        eprintln!("lease1: {lease1:?}");
        // Cannot acquire a 2nd lease while the first is are alive
        assert!(limit.acquire_lease_memory(&key).await.is_err());

        tokio::time::sleep(Duration::from_secs(1)).await;

        lease1.extend_memory(Duration::from_secs(6)).await.unwrap();

        // Wait for original lease duration to expire
        tokio::time::sleep(limit.duration + limit.duration).await;

        // Cannot acquire because we have an extended lease
        assert!(limit.acquire_lease_memory(&key).await.is_err());

        // Wait for extension to pass
        tokio::time::sleep(limit.duration + limit.duration).await;

        let _lease2 = limit.acquire_lease_memory(&key).await.unwrap();
    }

    #[tokio::test]
    async fn test_redis_extension() {
        if which::which("redis-server").is_err() {
            return;
        }
        let redis = RedisServer::spawn().await.unwrap();
        let conn = redis.connection().await.unwrap();

        let limit = LimitSpec {
            limit: 1,
            duration: Duration::from_secs(2),
        };

        let key = format!("test_redis-{}", Uuid::new_v4());
        let mut lease1 = limit.acquire_lease_redis(conn.clone(), &key).await.unwrap();
        eprintln!("lease1: {lease1:?}");
        // Cannot acquire a 2nd lease while the first is are alive
        assert!(limit.acquire_lease_redis(conn.clone(), &key).await.is_err());

        tokio::time::sleep(Duration::from_secs(1)).await;

        lease1
            .extend_redis(conn.clone(), Duration::from_secs(6))
            .await
            .unwrap();

        // Wait for original lease duration to expire
        tokio::time::sleep(limit.duration + limit.duration).await;

        // Cannot acquire because we have an extended lease
        assert!(limit.acquire_lease_redis(conn.clone(), &key).await.is_err());

        // Wait for extension to pass
        tokio::time::sleep(limit.duration + limit.duration).await;

        let mut lease2 = limit.acquire_lease_redis(conn.clone(), &key).await.unwrap();

        lease1.release_redis(conn.clone()).await;
        lease2.release_redis(conn.clone()).await;
    }
}
