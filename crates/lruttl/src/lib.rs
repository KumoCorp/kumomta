use kumo_server_memory::subscribe_to_memory_status_changes_async;
use moka::future::Cache;
use moka::policy::EvictionPolicy;
use moka::Expiry;
use parking_lot::Mutex;
use std::borrow::Borrow;
use std::future::Future;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};

static CACHES: LazyLock<Mutex<Vec<Weak<dyn CachePurger + Send + Sync>>>> =
    LazyLock::new(Mutex::default);

struct Inner<K: Clone + Hash + Eq, V: Clone + Send + Sync> {
    name: String,
    cache: Cache<K, Item<V>>,
}

trait CachePurger {
    fn name(&self) -> &str;
    fn purge(&self) -> usize;
}

impl<K: Clone + Send + Sync + Hash + Eq + 'static, V: Clone + Send + Sync + 'static> CachePurger
    for Inner<K, V>
{
    fn name(&self) -> &str {
        &self.name
    }
    fn purge(&self) -> usize {
        let num_entries = self.cache.entry_count();
        self.cache.invalidate_all();
        num_entries as usize
    }
}

pub fn purge_all_caches() {
    let mut purgers = vec![];
    {
        let mut caches = CACHES.lock();
        caches.retain(|entry| match entry.upgrade() {
            Some(purger) => {
                purgers.push(purger);
                true
            }
            None => false,
        })
    }

    tracing::error!("purging {} caches", purgers.len());
    for purger in purgers {
        let name = purger.name();
        let num_entries = purger.purge();
        tracing::error!("cleared {num_entries} entries from cache {name}");
    }
}

async fn prune_expired_caches() {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        let mut purgers = vec![];
        {
            let mut caches = CACHES.lock();
            caches.retain(|entry| match entry.upgrade() {
                Some(purger) => {
                    purgers.push(purger);
                    true
                }
                None => false,
            })
        }
    }
}

pub fn spawn_memory_monitor() {
    tokio::spawn(purge_caches_on_memory_shortage());
    tokio::spawn(prune_expired_caches());
}

async fn purge_caches_on_memory_shortage() {
    tracing::debug!("starting memory monitor");
    let mut memory_status = subscribe_to_memory_status_changes_async().await;
    while let Ok(()) = memory_status.changed().await {
        if kumo_server_memory::get_headroom() == 0 {
            purge_all_caches();

            // Wait a little bit so that we can debounce
            // in the case where we're riding the cusp of
            // the limit and would thrash the caches
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        }
    }
}

#[derive(Debug, Clone)]
struct Item<V>
where
    V: Send,
    V: Sync,
{
    item: V,
    expiration: Instant,
}

struct PerItemExpiry<K, V> {
    marker: PhantomData<(K, V)>,
}

impl<K: Send, V: Send + Sync> Expiry<K, Item<V>> for PerItemExpiry<K, V> {
    fn expire_after_create(
        &self,
        _key: &K,
        item: &Item<V>,
        created_at: Instant,
    ) -> Option<Duration> {
        Some(item.expiration - created_at)
    }
}

pub trait ItemTtl {
    fn get_ttl(&self) -> Duration;
}

pub struct LruCacheWithTtl<K: Clone + Hash + Eq, V: Clone + Send + Sync> {
    inner: Arc<Inner<K, V>>,
}

impl<
        K: Clone + Hash + Eq + Send + Sync + std::fmt::Debug + 'static,
        V: Clone + Send + Sync + 'static,
    > LruCacheWithTtl<K, V>
{
    #[deprecated = "use new_named instead"]
    pub fn new(capacity: usize) -> Self {
        Self::new_named("<anonymous>", capacity)
    }

    pub fn new_named<S: Into<String>>(name: S, capacity: usize) -> Self {
        let name = name.into();

        let cache = Cache::builder()
            .name(&name)
            .eviction_policy(EvictionPolicy::lru())
            .eviction_listener({
                let name = name.clone();
                move |k, _v, reason| {
                    tracing::trace!("evicting {name} {k:?} {reason:?}");
                }
            })
            .max_capacity(capacity as u64)
            .expire_after(PerItemExpiry {
                marker: PhantomData,
            })
            .build();

        let inner = Arc::new(Inner { name, cache });

        // Register with the global list of caches using a weak reference.
        // We need to "erase" the K/V types in order to do that, so we
        // use the CachePurger trait for this purpose.
        {
            let generic: Arc<dyn CachePurger + Send + Sync> = inner.clone();
            CACHES.lock().push(Arc::downgrade(&generic));
            tracing::debug!(
                "registered cache {} with capacity {capacity}",
                generic.name()
            );
        }

        Self { inner }
    }

    pub fn clear(&self) -> usize {
        let num_entries = self.inner.cache.entry_count();
        self.inner.cache.invalidate_all();
        num_entries as usize
    }

    pub async fn get_with_expiry<Q: ?Sized>(&self, name: &Q) -> Option<(V, Instant)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let entry = self.inner.cache.get(name).await?;
        Some((entry.item.clone(), entry.expiration))
    }

    pub async fn get<Q: ?Sized>(&self, name: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let entry = self.inner.cache.get(name).await?;
        entry.item.clone().into()
    }

    pub async fn insert(&self, name: K, item: V, expiration: Instant) -> V {
        self.inner
            .cache
            .insert(
                name,
                Item {
                    item: item.clone(),
                    expiration,
                },
            )
            .await;
        item
    }

    /// Get an existing item, but if that item doesn't already exist,
    /// execute the future `fut` to provide a value that will be inserted and then
    /// returned.  This is done atomically wrt. other callers.
    pub async fn get_or_insert(&self, name: K, ttl: Duration, fut: impl Future<Output = V>) -> V {
        let item = self
            .inner
            .cache
            .get_with(name, async move {
                let item = fut.await;
                Item {
                    item,
                    expiration: Instant::now() + ttl,
                }
            })
            .await;
        item.item
    }

    /// Get an existing item, but if that item doesn't already exist,
    /// execute the future `fut` to provide a value that will be inserted and then
    /// returned.  This is done atomically wrt. other callers.
    /// This variant allows the future to be fallible.
    pub async fn get_or_try_insert<E: Send + Sync + 'static>(
        &self,
        name: K,
        ttl: Duration,
        fut: impl Future<Output = Result<V, E>>,
    ) -> Result<V, Arc<E>> {
        let item = self
            .inner
            .cache
            .try_get_with(name, async move {
                let item = fut.await?;
                Ok(Item {
                    item,
                    expiration: Instant::now() + ttl,
                })
            })
            .await?;
        Ok(item.item)
    }

    /// Get an existing item, but if that item doesn't already exist,
    /// execute the future `fut` to provide a value that will be inserted and then
    /// returned.  This is done atomically wrt. other callers.
    /// This variant allows the future to be fallible, as well as returns a freshness
    /// flag that can be used to update cache metrics.
    /// if the freshness flag is true, it was a cache miss.
    pub async fn get_or_try_insert_with_freshness_status<E: Send + Sync + 'static>(
        &self,
        name: &K,
        ttl: Duration,
        fut: impl Future<Output = Result<V, E>>,
    ) -> Result<(bool, V), Arc<E>> {
        let entry = self
            .inner
            .cache
            .entry_by_ref(name)
            .or_try_insert_with(async move {
                let item = fut.await?;
                Ok(Item {
                    item,
                    expiration: Instant::now() + ttl,
                })
            })
            .await?;
        let is_fresh = entry.is_fresh();
        Ok((is_fresh, entry.value().item.clone()))
    }
}

impl<
        K: Clone + Hash + Eq + Send + Sync + std::fmt::Debug + 'static,
        V: Clone + Send + Sync + ItemTtl + 'static,
    > LruCacheWithTtl<K, V>
{
    pub async fn get_or_try_insert_embedded_ttl<E: Send + Sync + 'static>(
        &self,
        name: K,
        fut: impl Future<Output = Result<V, E>>,
    ) -> Result<V, Arc<E>> {
        let item = self
            .inner
            .cache
            .try_get_with(name, async move {
                let item = fut.await?;
                let ttl = item.get_ttl();
                Ok(Item {
                    item,
                    expiration: Instant::now() + ttl,
                })
            })
            .await?;
        Ok(item.item)
    }
}
