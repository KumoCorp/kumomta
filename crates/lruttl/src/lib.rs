use dashmap::DashMap;
use kumo_server_memory::subscribe_to_memory_status_changes_async;
use parking_lot::Mutex;
use prometheus::{IntCounter, IntCounterVec, IntGauge, IntGaugeVec};
use std::borrow::Borrow;
use std::collections::HashSet;
use std::fmt::Debug;
use std::future::Future;
use std::hash::Hash;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Weak};
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration, Instant};
pub use {linkme, paste};

static CACHE_LOOKUP: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_lookup_count",
        "how many times a lruttl cache lookup was initiated for a given cache",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_EVICT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_evict_count",
        "how many times a lruttl cache evicted an item due to capacity constraints",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_EXPIRE: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_expire_count",
        "how many times a lruttl cache removed an item due to ttl expiration",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_HIT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_hit_count",
        "how many times a lruttl cache lookup was a hit for a given cache",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_MISS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_miss_count",
        "how many times a lruttl cache lookup was a miss for a given cache",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_INSERT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_insert_count",
        "how many times a lruttl cache was populated via unconditional insert",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_POPULATED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_populated_count",
        "how many times a lruttl cache lookup resulted in performing the work to populate the entry",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_ERROR: LazyLock<IntCounterVec> = LazyLock::new(|| {
    prometheus::register_int_counter_vec!(
        "lruttl_error_count",
        "how many times a lruttl cache population resulted in an error",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_WAIT: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "lruttl_waiting_populate",
        "how many tasks are currently waiting for a cache entry to populate",
        &["cache_name"]
    )
    .unwrap()
});
static CACHE_SIZE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    prometheus::register_int_gauge_vec!(
        "lruttl_cache_size",
        "number of items contained in an lruttl cache",
        &["cache_name"]
    )
    .unwrap()
});

static CACHES: LazyLock<Mutex<Vec<Weak<dyn CachePurger + Send + Sync>>>> =
    LazyLock::new(Mutex::default);

struct Inner<K: Clone + Hash + Eq + Debug, V: Clone + Send + Sync + Debug> {
    name: String,
    tick: AtomicUsize,
    capacity: usize,
    cache: DashMap<K, Item<V>>,
    lru_samples: AtomicUsize,
    lookup_counter: IntCounter,
    evict_counter: IntCounter,
    expire_counter: IntCounter,
    hit_counter: IntCounter,
    miss_counter: IntCounter,
    populate_counter: IntCounter,
    insert_counter: IntCounter,
    error_counter: IntCounter,
    wait_gauge: IntGauge,
    size_gauge: IntGauge,
}

trait CachePurger {
    fn name(&self) -> &str;
    fn purge(&self) -> usize;
    fn process_expirations(&self) -> usize;
}

impl<
        K: Clone + Debug + Send + Sync + Hash + Eq + 'static,
        V: Clone + Debug + Send + Sync + 'static,
    > CachePurger for Inner<K, V>
{
    fn name(&self) -> &str {
        &self.name
    }
    fn purge(&self) -> usize {
        let num_entries = self.cache.len();
        self.cache.clear();
        self.size_gauge.set(self.cache.len() as i64);
        num_entries
    }
    fn process_expirations(&self) -> usize {
        let now = Instant::now();
        let mut expired_keys = vec![];
        for map_entry in self.cache.iter() {
            let item = map_entry.value();
            match &item.item {
                ItemState::Pending(_) => {
                    // Cannot evict a pending lookup
                }
                ItemState::Present(_) | ItemState::Failed(_) => {
                    if now >= item.expiration {
                        expired_keys.push(map_entry.key().clone());
                    }
                }
            }
        }

        let mut num_removed = 0;
        for key in expired_keys {
            // Sanity check that it is still expired before removing it,
            // because it would be a shame to remove it if another actor
            // has just updated it
            let removed = self
                .cache
                .remove_if(&key, |_k, entry| now >= entry.expiration)
                .is_some();
            if removed {
                num_removed += 1;
                self.expire_counter.inc();
                self.size_gauge.set(self.cache.len() as i64);
            }
        }
        num_removed
    }
}

fn all_caches() -> Vec<Arc<dyn CachePurger + Send + Sync>> {
    let mut result = vec![];
    let mut caches = CACHES.lock();
    caches.retain(|entry| match entry.upgrade() {
        Some(purger) => {
            result.push(purger);
            true
        }
        None => false,
    });
    result
}

pub fn purge_all_caches() {
    let purgers = all_caches();

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
        let purgers = all_caches();

        for p in purgers {
            let n = p.process_expirations();
            if n > 0 {
                tracing::debug!("expired {n} entries from cache {}", p.name());
            }
        }
    }
}

#[linkme::distributed_slice]
pub static LRUTTL_VIVIFY: [fn() -> &'static str];

/// Declare a cache as a static, and link it into the list of possible
/// pre-defined caches.
///
/// Due to a limitation in implementation details, you must also add
/// `linkme.workspace = true` to the manifest of the crate where you
/// use this macro.
#[macro_export]
macro_rules! declare_cache {
    ($vis:vis
        static $sym:ident:
        LruCacheWithTtl<$key:ty, $value:ty>::new($name:expr, $capacity:expr);
    ) => {
        $vis static $sym: ::std::sync::LazyLock<$crate::LruCacheWithTtl<$key, $value>> =
            ::std::sync::LazyLock::new(
                || $crate::LruCacheWithTtl::new($name, $capacity));

        // Link into LRUTTL_VIVIFY
        $crate::paste::paste! {
            #[linkme::distributed_slice($crate::LRUTTL_VIVIFY)]
            static [<VIVIFY_ $sym>]: fn() -> &'static str = || {
                ::std::sync::LazyLock::force(&$sym);
                $name
            };
        }
    };
}

/// Ensure that all caches declared via declare_cache!
/// have been instantiated and returns the set of names.
fn vivify() {
    LazyLock::force(&PREDEFINED_NAMES);
}

fn vivify_impl() -> HashSet<&'static str> {
    let mut set = HashSet::new();

    for vivify_func in LRUTTL_VIVIFY {
        let name = vivify_func();
        assert!(!set.contains(name), "duplicate cache name {name}");
        set.insert(name);
    }

    set
}

static PREDEFINED_NAMES: LazyLock<HashSet<&'static str>> = LazyLock::new(vivify_impl);

pub fn is_name_available(name: &str) -> bool {
    !PREDEFINED_NAMES.contains(name)
}

pub fn spawn_memory_monitor() {
    vivify();
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
enum ItemState<V>
where
    V: Send,
    V: Sync,
{
    Present(V),
    Pending(Arc<Semaphore>),
    Failed(Arc<anyhow::Error>),
}

#[derive(Debug)]
struct Item<V>
where
    V: Send,
    V: Sync,
{
    item: ItemState<V>,
    expiration: Instant,
    last_tick: AtomicUsize,
}

impl<V: Clone + Send + Sync> Clone for Item<V> {
    fn clone(&self) -> Self {
        Self {
            item: self.item.clone(),
            expiration: self.expiration,
            last_tick: self.last_tick.load(Ordering::Relaxed).into(),
        }
    }
}

#[derive(Debug)]
pub struct ItemLookup<V: Debug> {
    /// A copy of the item
    pub item: V,
    /// If true, the get_or_try_insert operation populated the entry;
    /// the operation was a cache miss
    pub is_fresh: bool,
    /// The instant at which this entry will expire
    pub expiration: Instant,
}

pub struct LruCacheWithTtl<K: Clone + Debug + Hash + Eq, V: Clone + Debug + Send + Sync> {
    inner: Arc<Inner<K, V>>,
}

impl<
        K: Clone + Debug + Hash + Eq + Send + Sync + std::fmt::Debug + 'static,
        V: Clone + Debug + Send + Sync + 'static,
    > LruCacheWithTtl<K, V>
{
    pub fn new<S: Into<String>>(name: S, capacity: usize) -> Self {
        let name = name.into();
        let cache = DashMap::new();

        let lookup_counter = CACHE_LOOKUP
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");
        let hit_counter = CACHE_HIT
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");
        let evict_counter = CACHE_EVICT
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");
        let expire_counter = CACHE_EXPIRE
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");
        let miss_counter = CACHE_MISS
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");
        let populate_counter = CACHE_POPULATED
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");
        let insert_counter = CACHE_INSERT
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");
        let error_counter = CACHE_ERROR
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");
        let wait_gauge = CACHE_WAIT
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");
        let size_gauge = CACHE_SIZE
            .get_metric_with_label_values(&[&name])
            .expect("failed to get counter");

        let inner = Arc::new(Inner {
            name,
            cache,
            tick: AtomicUsize::new(0),
            capacity,
            lru_samples: AtomicUsize::new(10),
            lookup_counter,
            evict_counter,
            expire_counter,
            hit_counter,
            miss_counter,
            populate_counter,
            error_counter,
            wait_gauge,
            insert_counter,
            size_gauge,
        });

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
        let num_entries = self.inner.cache.len();
        self.inner.cache.clear();
        self.inner.size_gauge.set(self.inner.cache.len() as i64);
        num_entries
    }

    fn inc_tick(&self) -> usize {
        self.inner.tick.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn update_tick(&self, item: &Item<V>) {
        let v = self.inc_tick();
        item.last_tick.store(v, Ordering::Relaxed);
    }

    /// Evict up to target entries.
    ///
    /// We use a probablistic approach to the LRU, because
    /// it is challenging to safely thread the classic doubly-linked-list
    /// through dashmap.
    ///
    /// target is bounded to half of number of selected samples, in
    /// order to ensure that we don't randomly pick the newest element
    /// from the set when under pressure.
    ///
    /// Redis uses a similar technique for its LRU as described
    /// in <https://redis.io/docs/latest/develop/reference/eviction/#apx-lru>
    /// which suggests that sampling 10 keys at random to them compare
    /// their recency yields a reasonably close approximation to the
    /// 100% precise LRU.
    ///
    /// Since we also support TTLs, we'll just go ahead and remove
    /// any expired keys that show up in the sampled set.
    fn evict_some(&self, target: usize) -> bool {
        let now = Instant::now();

        // Approximate (since it could change immediately after reading)
        // cache size
        let cache_size = self.inner.cache.len();
        // How many keys to sample
        let num_samples = self
            .inner
            .lru_samples
            .load(Ordering::Relaxed)
            .min(cache_size);

        // a list of keys which have expired
        let mut expired_keys = vec![];
        // a random selection of up to num_samples (key, tick) tuples
        let mut samples = vec![];

        // Pick some random keys.
        // The rand crate has some helpers for working with iterators,
        // but they appear to copy many elements into an internal buffer
        // in order to make a selection, and we want to avoid directly
        // considering every possible element because some users have
        // very large capacity caches.
        //
        // The approach taken here is to produce a random list of iterator
        // offsets so that we can skim across the iterator in a single
        // pass and pull out a random selection of elements.
        // The sample function provides a randomized list of indices that
        // we can use for this; we need to sort it first, but the cost
        // should be reasonably low as num_samples should be ~10 or so
        // in the most common configuration.
        {
            let mut rng = rand::thread_rng();
            let mut indices =
                rand::seq::index::sample(&mut rng, cache_size, num_samples).into_vec();
            indices.sort();
            let mut iter = self.inner.cache.iter();
            let mut current_idx = 0;

            /// Advance an iterator by skip_amount.
            /// Ideally we'd use Iterator::advance_by for this, but at the
            /// time of writing that method is nightly only.
            /// Note that it also uses next() internally anyway
            fn advance_by(iter: &mut impl Iterator, skip_amount: usize) {
                for _ in 0..skip_amount {
                    if iter.next().is_none() {
                        return;
                    }
                }
            }

            for idx in indices {
                // idx is the index we want to be on; we'll need to skip ahead
                // by some number of slots based on the current one. skip_amount
                // is that number.
                let skip_amount = idx - current_idx;
                advance_by(&mut iter, skip_amount);

                match iter.next() {
                    Some(map_entry) => {
                        current_idx = idx + 1;
                        let item = map_entry.value();
                        match &item.item {
                            ItemState::Pending(_) => {
                                // Cannot evict a pending lookup
                            }
                            ItemState::Present(_) | ItemState::Failed(_) => {
                                if now >= item.expiration {
                                    expired_keys.push(map_entry.key().clone());
                                } else {
                                    let last_tick = item.last_tick.load(Ordering::Relaxed);
                                    samples.push((map_entry.key().clone(), last_tick));
                                }
                            }
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
        }

        let mut num_removed = 0;
        for key in expired_keys {
            // Sanity check that it is still expired before removing it,
            // because it would be a shame to remove it if another actor
            // has just updated it
            let removed = self
                .inner
                .cache
                .remove_if(&key, |_k, entry| now >= entry.expiration)
                .is_some();
            if removed {
                tracing::trace!("{} expired {key:?}", self.inner.name);
                num_removed += 1;
                self.inner.expire_counter.inc();
            }
        }

        // Since we're picking random elements, we want to ensure that
        // we never pick the newest element from the set to evict because
        // that is likely the wrong choice. We need enough samples to
        // know that the lowest number we picked is representative
        // of the eldest element in the map overall.
        // We limit ourselves to half of the number of selected samples.
        let target = target.min(samples.len() / 2).max(1);

        // If we met our target, skip the extra work below
        if num_removed >= target {
            self.inner.size_gauge.set(self.inner.cache.len() as i64);
            tracing::trace!(
                "{} expired {num_removed} of target {target}",
                self.inner.name
            );
            return true;
        }

        // Sort by ascending tick, which is equivalent to having the
        // LRU within that set towards the front of the vec
        samples.sort_by(|(_ka, tick_a), (_kb, tick_b)| tick_a.cmp(&tick_b));

        for (key, tick) in samples {
            // Sanity check that the tick value is the same as we expect.
            // If it has changed since we sampled it, then that element
            // is no longer a good candidate for LRU eviction.
            if self
                .inner
                .cache
                .remove_if(&key, |_k, item| {
                    item.last_tick.load(Ordering::Relaxed) == tick
                })
                .is_some()
            {
                tracing::debug!("{} evicted {key:?}", self.inner.name);
                num_removed += 1;
                self.inner.evict_counter.inc();
                self.inner.size_gauge.set(self.inner.cache.len() as i64);
                if num_removed >= target {
                    return true;
                }
            }
        }

        if num_removed == 0 {
            tracing::warn!(
                "{} did not find anything to evict, target was {target}",
                self.inner.name
            );
        }

        tracing::trace!(
            "{} removed {num_removed} of target {target}",
            self.inner.name
        );

        num_removed > 0
    }

    /// Potentially make some progress to get back under
    /// budget on the cache capacity
    fn maybe_evict(&self) {
        let cache_size = self.inner.cache.len();
        if cache_size > self.inner.capacity {
            self.evict_some(cache_size - self.inner.capacity);
        }
    }

    pub async fn lookup<Q: ?Sized>(&self, name: &Q) -> Option<ItemLookup<V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.inner.lookup_counter.inc();
        match self.inner.cache.get_mut(name) {
            None => {
                self.inner.miss_counter.inc();
                return None;
            }
            Some(entry) => {
                match &entry.item {
                    ItemState::Present(item) => {
                        let now = Instant::now();
                        if now >= entry.expiration {
                            // Expired; remove it from the map.
                            // Take care to drop our ref first so that we don't
                            // self-deadlock
                            drop(entry);
                            if self
                                .inner
                                .cache
                                .remove_if(name, |_k, entry| now >= entry.expiration)
                                .is_some()
                            {
                                self.inner.expire_counter.inc();
                                self.inner.size_gauge.set(self.inner.cache.len() as i64);
                            }
                            self.inner.miss_counter.inc();
                            return None;
                        }
                        self.inner.hit_counter.inc();
                        self.update_tick(&entry);
                        Some(ItemLookup {
                            item: item.clone(),
                            expiration: entry.expiration,
                            is_fresh: false,
                        })
                    }
                    ItemState::Pending(_) | ItemState::Failed(_) => {
                        self.inner.miss_counter.inc();
                        None
                    }
                }
            }
        }
    }

    pub async fn get<Q: ?Sized>(&self, name: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.lookup(name).await.map(|lookup| lookup.item)
    }

    pub async fn insert(&self, name: K, item: V, expiration: Instant) -> V {
        self.inner.cache.insert(
            name,
            Item {
                item: ItemState::Present(item.clone()),
                expiration,
                last_tick: self.inc_tick().into(),
            },
        );

        self.inner.insert_counter.inc();
        self.inner.size_gauge.set(self.inner.cache.len() as i64);
        self.maybe_evict();

        item
    }

    fn clone_item_state(&self, name: &K) -> (ItemState<V>, Instant) {
        let mut is_new = false;
        let mut entry = self.inner.cache.entry(name.clone()).or_insert_with(|| {
            is_new = true;
            Item {
                item: ItemState::Pending(Arc::new(Semaphore::new(1))),
                expiration: Instant::now() + Duration::from_secs(60),
                last_tick: self.inc_tick().into(),
            }
        });

        match &entry.value().item {
            ItemState::Pending(_) => {}
            ItemState::Present(_) | ItemState::Failed(_) => {
                let now = Instant::now();
                if now >= entry.expiration {
                    // Expired; we will need to fetch it
                    entry.value_mut().item = ItemState::Pending(Arc::new(Semaphore::new(1)));
                }
            }
        }

        self.update_tick(&entry);
        let item = entry.value();
        let result = (item.item.clone(), entry.expiration);
        drop(entry);

        if is_new {
            self.inner.size_gauge.set(self.inner.cache.len() as i64);
            self.maybe_evict();
        }

        result
    }

    /// Get an existing item, but if that item doesn't already exist,
    /// execute the future `fut` to provide a value that will be inserted and then
    /// returned.  This is done atomically wrt. other callers.
    /// The TTL parameter is a function that can extract the TTL from the value type,
    /// or just return a constant TTL.
    pub async fn get_or_try_insert<E: Into<anyhow::Error>, TTL: FnOnce(&V) -> Duration>(
        &self,
        name: &K,
        ttl_func: TTL,
        fut: impl Future<Output = Result<V, E>>,
    ) -> Result<ItemLookup<V>, Arc<anyhow::Error>> {
        // Fast path avoids cloning the key
        if let Some(entry) = self.lookup(name).await {
            return Ok(entry);
        }

        // Note: the lookup call increments lookup_counter and miss_counter

        match self.clone_item_state(name) {
            (ItemState::Present(item), expiration) => {
                return Ok(ItemLookup {
                    item,
                    expiration,
                    is_fresh: false,
                });
            }
            (ItemState::Failed(error), _) => {
                return Err(error);
            }
            (ItemState::Pending(sema), _) => {
                /// A little helper to ensure that we decrement the count
                /// when we unwind, in the case that this future is cancelled
                /// or abandoned prior to completion
                struct DecOnDrop(IntGauge);
                impl DecOnDrop {
                    /// Increment on acquire, decrement on drop
                    fn new(g: IntGauge) -> Self {
                        g.inc();
                        Self(g)
                    }
                }
                impl Drop for DecOnDrop {
                    fn drop(&mut self) {
                        self.0.dec();
                    }
                }

                let wait_count = DecOnDrop::new(self.inner.wait_gauge.clone());
                let wait_result =
                    match timeout(Duration::from_secs(120), sema.acquire_owned()).await {
                        Err(_) => {
                            self.inner.error_counter.inc();
                            tracing::error!(
                                "{} semaphore acquire for {name:?} timed out",
                                self.inner.name
                            );
                            return Err(Arc::new(anyhow::anyhow!(
                                "{} lookup for {name:?} \
                                            timed out on semaphore acquire",
                                self.inner.name
                            )));
                        }
                        Ok(r) => r,
                    };

                drop(wait_count);

                // While we slept, someone else may have satisfied
                // the lookup; check it
                match self.clone_item_state(name) {
                    (ItemState::Present(item), expiration) => {
                        return Ok(ItemLookup {
                            item,
                            expiration,
                            is_fresh: false,
                        });
                    }
                    (ItemState::Failed(error), _) => {
                        self.inner.hit_counter.inc();
                        return Err(error);
                    }
                    (ItemState::Pending(current_sema), _) => {
                        // It's still outstanding
                        match wait_result {
                            Ok(permit) => {
                                // We're responsible for resolving it

                                if !Arc::ptr_eq(&current_sema, permit.semaphore()) {
                                    self.inner.error_counter.inc();
                                    tracing::error!(
                                        "{} mismatched semaphores for {name:?}",
                                        self.inner.name
                                    );

                                    // sema is the one we started with, and
                                    // we own the permit for it. Both us and
                                    // anyone else waiting for this is going
                                    // to be let down by this situation.
                                    permit.semaphore().close();
                                    return Err(Arc::new(anyhow::anyhow!(
                                        "{} lookup for {name:?} \
                                            but have mismatched semaphores",
                                        self.inner.name
                                    )));
                                }

                                self.inner.populate_counter.inc();
                                let mut ttl = Duration::from_secs(60);
                                let future_result = fut.await;
                                let now = Instant::now();

                                let (item_result, return_value) = match future_result {
                                    Ok(item) => {
                                        ttl = ttl_func(&item);
                                        (
                                            ItemState::Present(item.clone()),
                                            Ok(ItemLookup {
                                                item,
                                                expiration: now + ttl,
                                                is_fresh: true,
                                            }),
                                        )
                                    }
                                    Err(err) => {
                                        self.inner.error_counter.inc();
                                        let err = Arc::new(err.into());
                                        (ItemState::Failed(err.clone()), Err(err))
                                    }
                                };

                                self.inner.cache.insert(
                                    name.clone(),
                                    Item {
                                        item: item_result,
                                        expiration: Instant::now() + ttl,
                                        last_tick: self.inc_tick().into(),
                                    },
                                );
                                // Wake everybody up
                                permit.semaphore().close();
                                self.maybe_evict();

                                return return_value;
                            }
                            Err(_) => {
                                self.inner.error_counter.inc();

                                // semaphore was closed, but the status is
                                // still somehow pending
                                tracing::error!(
                                    "{} lookup for {name:?} woke up semas \
                                    but is still marked pending",
                                    self.inner.name
                                );
                                return Err(Arc::new(anyhow::anyhow!(
                                    "{} lookup for {name:?} \
                                            sema was closed but state is still pending",
                                    self.inner.name
                                )));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use test_log::test; // run with RUST_LOG=lruttl=trace to trace

    #[test(tokio::test)]
    async fn test_capacity() {
        let cache = LruCacheWithTtl::new("test_capacity", 40);

        let expiration = Instant::now() + Duration::from_secs(60);
        for i in 0..100 {
            cache.insert(i, i, expiration).await;
        }

        assert_eq!(cache.inner.cache.len(), 40, "capacity is respected");
    }

    #[test(tokio::test)]
    async fn test_expiration() {
        let cache = LruCacheWithTtl::new("test_expiration", 1);

        tokio::time::pause();
        let expiration = Instant::now() + Duration::from_secs(1);
        cache.insert(0, 0, expiration).await;

        cache.get(&0).await.expect("still in cache");
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(cache.get(&0).await.is_none(), "evicted due to ttl");
    }
}
