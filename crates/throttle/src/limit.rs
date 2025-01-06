use crate::{Error, LimitSpec, REDIS};
use anyhow::{anyhow, Context};
use mod_redis::{RedisConnection, Script};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Notify;
use uuid::Uuid;

static MEMORY: LazyLock<Mutex<MemoryStore>> = LazyLock::new(|| Mutex::new(MemoryStore::new()));

static ACQUIRE_SCRIPT: LazyLock<Script> = LazyLock::new(|| Script::new(include_str!("limit.lua")));

pub struct LimitSpecWithDuration {
    pub spec: LimitSpec,
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

impl LimitSpecWithDuration {
    pub async fn acquire_lease<S: AsRef<str>>(
        &self,
        key: S,
        deadline: Instant,
    ) -> Result<LimitLease, Error> {
        match (self.spec.force_local, REDIS.get()) {
            (false, Some(redis)) => {
                self.acquire_lease_redis(&redis, key.as_ref(), deadline)
                    .await
            }
            (true, _) | (false, None) => self.acquire_lease_memory(key.as_ref(), deadline).await,
        }
    }

    pub async fn acquire_lease_redis(
        &self,
        conn: &RedisConnection,
        key: &str,
        deadline: Instant,
    ) -> Result<LimitLease, Error> {
        loop {
            let now_ts = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);

            let expires_ts = now_ts + self.duration.as_secs_f64();
            let uuid = Uuid::new_v4();
            let uuid_str = uuid.to_string();

            let mut script = ACQUIRE_SCRIPT.prepare_invoke();
            script
                .key(key)
                .arg(now_ts)
                .arg(expires_ts)
                .arg(self.spec.limit)
                .arg(uuid_str);

            match conn.invoke_script(script).await.with_context(|| {
                format!(
                    "error invoking redis lease acquisition script \
                     key={key} now={now_ts} expires={expires_ts} \
                     limit={} uuid={uuid}",
                    self.spec.limit
                )
            })? {
                mod_redis::RedisValue::Okay => {
                    return Ok(LimitLease {
                        name: key.to_string(),
                        uuid,
                        armed: true,
                        backend: Backend::Redis,
                    });
                }
                mod_redis::RedisValue::Int(next_expiration_interval) => {
                    if Instant::now() >= deadline {
                        return Err(Error::TooManyLeases(Duration::from_secs(
                            next_expiration_interval as u64,
                        )));
                    }

                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
                mod_redis::RedisValue::Double(next_expiration_interval) => {
                    if Instant::now() >= deadline {
                        return Err(Error::TooManyLeases(Duration::from_secs(
                            next_expiration_interval as u64,
                        )));
                    }

                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
                value => {
                    return Err(anyhow!("acquire script succeeded but returned {value:?}").into());
                }
            }
        }
    }

    pub async fn acquire_lease_memory(
        &self,
        key: &str,
        deadline: Instant,
    ) -> Result<LimitLease, Error> {
        let uuid = Uuid::new_v4();

        fn resolve_set(key: &str) -> Arc<LeaseSet> {
            MEMORY.lock().get_or_create(key)
        }

        let set = resolve_set(key);

        set.acquire(uuid, self.spec.limit, self.duration, deadline)
            .await?;

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
                if let Some(redis) = REDIS.get() {
                    self.release_redis(&redis).await;
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
                if let Some(redis) = REDIS.get() {
                    self.extend_redis(&redis, duration).await
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
        let store = MEMORY.lock();
        if let Some(set) = store.get(&self.name) {
            set.extend(self.uuid, duration)
        } else {
            Err(Error::NonExistentLease)
        }
    }

    async fn extend_redis(&self, conn: &RedisConnection, duration: Duration) -> Result<(), Error> {
        let now_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        let expires = now_ts + duration.as_secs_f64();

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

    async fn release_memory(&self) {
        let store = MEMORY.lock();
        if let Some(set) = store.get(&self.name) {
            set.release(self.uuid);
        }
    }

    async fn release_redis(&mut self, conn: &RedisConnection) {
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
    members: Mutex<HashMap<Uuid, Instant>>,
    notify: Notify,
}

impl LeaseSet {
    fn new() -> Self {
        Self {
            members: Mutex::new(HashMap::new()),
            notify: Notify::new(),
        }
    }

    fn acquire_immediate(&self, uuid: Uuid, limit: u64, duration: Duration) -> bool {
        let mut members = self.members.lock();
        let now = Instant::now();
        members.retain(|_k, expiry| *expiry > now);

        if members.len() as u64 + 1 <= limit {
            members.insert(uuid, now + duration);
            return true;
        }

        false
    }

    async fn acquire(
        &self,
        uuid: Uuid,
        limit: u64,
        duration: Duration,
        deadline: Instant,
    ) -> Result<(), Error> {
        loop {
            if self.acquire_immediate(uuid, limit, duration) {
                return Ok(());
            }

            match tokio::time::timeout_at(deadline.into(), self.notify.notified()).await {
                Err(_) => {
                    if self.acquire_immediate(uuid, limit, duration) {
                        return Ok(());
                    }
                    let min_expiration = self
                        .members
                        .lock()
                        .values()
                        .cloned()
                        .min()
                        .expect("some elements");
                    return Err(Error::TooManyLeases(min_expiration - Instant::now()));
                }
                Ok(_) => {
                    // Try to acquire again
                    continue;
                }
            }
        }
    }

    fn extend(&self, uuid: Uuid, duration: Duration) -> Result<(), Error> {
        match self.members.lock().get_mut(&uuid) {
            Some(entry) => {
                *entry = Instant::now() + duration;
                Ok(())
            }
            None => Err(Error::NonExistentLease),
        }
    }

    fn release(&self, uuid: Uuid) {
        let mut members = self.members.lock();
        members.remove(&uuid);
        self.notify.notify_one();
    }
}

struct MemoryStore {
    sets: HashMap<String, Arc<LeaseSet>>,
}

impl MemoryStore {
    fn new() -> Self {
        Self {
            sets: HashMap::new(),
        }
    }

    fn get(&self, name: &str) -> Option<Arc<LeaseSet>> {
        self.sets.get(name).map(Arc::clone)
    }

    fn get_or_create(&mut self, name: &str) -> Arc<LeaseSet> {
        self.sets
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(LeaseSet::new()))
            .clone()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use mod_redis::test::{RedisCluster, RedisServer};

    #[tokio::test]
    async fn test_memory() {
        let limit = LimitSpecWithDuration {
            spec: LimitSpec::new(2),
            duration: Duration::from_secs(2),
        };

        let key = format!("test_memory-{}", Uuid::new_v4());
        let lease1 = limit
            .acquire_lease_memory(&key, Instant::now())
            .await
            .unwrap();
        eprintln!("lease1: {lease1:?}");
        let mut lease2 = limit
            .acquire_lease_memory(&key, Instant::now())
            .await
            .unwrap();
        eprintln!("lease2: {lease2:?}");
        // Cannot acquire a 3rd lease while the other two are alive
        assert!(limit
            .acquire_lease_memory(&key, Instant::now())
            .await
            .is_err());

        // Release and try to get a third
        lease2.release().await;
        let _lease3 = limit
            .acquire_lease_memory(&key, Instant::now())
            .await
            .unwrap();

        // Cannot acquire while the other two are alive
        assert!(limit
            .acquire_lease_memory(&key, Instant::now())
            .await
            .is_err());

        let start = Instant::now();

        // We can acquire another after waiting for some number of leases to expire
        let _lease4 = limit
            .acquire_lease_memory(&key, start + limit.duration + limit.duration)
            .await
            .unwrap();

        assert!(
            start.elapsed() > limit.duration,
            "elapsed is {:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn test_redis() {
        if !RedisServer::is_available() {
            return;
        }
        let redis = RedisServer::spawn("").await.unwrap();
        let conn = redis.connection().await.unwrap();

        let limit = LimitSpecWithDuration {
            spec: LimitSpec::new(2),
            duration: Duration::from_secs(2),
        };

        let key = format!("test_redis-{}", Uuid::new_v4());
        let mut lease1 = limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .unwrap();
        eprintln!("lease1: {lease1:?}");
        let mut lease2 = limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .unwrap();
        eprintln!("lease2: {lease2:?}");
        // Cannot acquire a 3rd lease while the other two are alive
        assert!(limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .is_err());

        // Release and try to get a third
        lease2.release_redis(&conn).await;
        let mut lease3 = limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .unwrap();

        // Cannot acquire while the other two are alive
        assert!(limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .is_err());

        let start = Instant::now();

        // We can acquire another after waiting for some number of leases to expire
        let mut lease4 = limit
            .acquire_lease_redis(&conn, &key, start + limit.duration + limit.duration)
            .await
            .unwrap();

        assert!(
            start.elapsed() > limit.duration,
            "elapsed is {:?}",
            start.elapsed()
        );

        lease1.release_redis(&conn).await;
        lease3.release_redis(&conn).await;
        lease4.release_redis(&conn).await;
    }

    #[tokio::test]
    async fn test_redis_cluster() {
        if !RedisCluster::is_available().await {
            return;
        }
        let redis = RedisCluster::spawn().await.unwrap();
        let conn = redis.connection().await.unwrap();

        let limit = LimitSpecWithDuration {
            spec: LimitSpec::new(2),
            duration: Duration::from_secs(2),
        };

        let key = format!("test_redis-{}", Uuid::new_v4());
        let mut lease1 = limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .unwrap();
        eprintln!("lease1: {lease1:?}");
        let mut lease2 = limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .unwrap();
        eprintln!("lease2: {lease2:?}");
        // Cannot acquire a 3rd lease while the other two are alive
        assert!(limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .is_err());

        // Release and try to get a third
        lease2.release_redis(&conn).await;
        let mut lease3 = limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .unwrap();

        // Cannot acquire while the other two are alive
        assert!(limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .is_err());

        // Wait for some number of leases to expire
        tokio::time::sleep(limit.duration + limit.duration).await;

        let mut lease4 = limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .unwrap();

        lease1.release_redis(&conn).await;
        lease3.release_redis(&conn).await;
        lease4.release_redis(&conn).await;
    }

    #[tokio::test]
    async fn test_memory_extension() {
        let limit = LimitSpecWithDuration {
            spec: LimitSpec::new(1),
            duration: Duration::from_secs(2),
        };

        let key = format!("test_redis-{}", Uuid::new_v4());
        let lease1 = limit
            .acquire_lease_memory(&key, Instant::now())
            .await
            .unwrap();
        eprintln!("lease1: {lease1:?}");
        // Cannot acquire a 2nd lease while the first is are alive
        assert!(limit
            .acquire_lease_memory(&key, Instant::now())
            .await
            .is_err());

        tokio::time::sleep(Duration::from_secs(1)).await;

        lease1.extend_memory(Duration::from_secs(6)).await.unwrap();

        // Wait for original lease duration to expire
        tokio::time::sleep(limit.duration + limit.duration).await;

        // Cannot acquire because we have an extended lease
        assert!(limit
            .acquire_lease_memory(&key, Instant::now())
            .await
            .is_err());

        // Wait for extension to pass
        tokio::time::sleep(limit.duration + limit.duration).await;

        let _lease2 = limit
            .acquire_lease_memory(&key, Instant::now())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_redis_extension() {
        if !RedisServer::is_available() {
            return;
        }
        let redis = RedisServer::spawn("").await.unwrap();
        let conn = redis.connection().await.unwrap();

        let limit = LimitSpecWithDuration {
            spec: LimitSpec::new(1),
            duration: Duration::from_secs(2),
        };

        let key = format!("test_redis-{}", Uuid::new_v4());
        let mut lease1 = limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .unwrap();
        eprintln!("lease1: {lease1:?}");
        // Cannot acquire a 2nd lease while the first is are alive
        assert!(limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .is_err());

        tokio::time::sleep(Duration::from_secs(1)).await;

        lease1
            .extend_redis(&conn, Duration::from_secs(6))
            .await
            .unwrap();

        // Wait for original lease duration to expire
        tokio::time::sleep(limit.duration + limit.duration).await;

        // Cannot acquire because we have an extended lease
        assert!(limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .is_err());

        // Wait for extension to pass
        tokio::time::sleep(limit.duration + limit.duration).await;

        let mut lease2 = limit
            .acquire_lease_redis(&conn, &key, Instant::now())
            .await
            .unwrap();

        lease1.release_redis(&conn).await;
        lease2.release_redis(&conn).await;
    }
}
