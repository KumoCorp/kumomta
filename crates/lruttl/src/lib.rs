/*
 * This file is derived from code which is:
 * Copyright (c) 2020-2023, Stalwart Labs Ltd.
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
 * option. This file may not be copied, modified, or distributed
 * except according to those terms.
 */

use kumo_server_memory::subscribe_to_memory_status_changes_async;
use lru_cache::LruCache;
use parking_lot::Mutex;
use std::borrow::Borrow;
use std::hash::Hash;
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};

static CACHES: LazyLock<Mutex<Vec<Weak<dyn CachePurger + Send + Sync>>>> =
    LazyLock::new(Mutex::default);

struct Inner<K: Clone + Hash + Eq, V: Clone> {
    name: String,
    cache: Mutex<LruCache<K, Item<V>>>,
}

trait CachePurger {
    fn name(&self) -> &str;
    fn purge(&self) -> usize;
    fn prune_expired(&self) -> usize;
}

impl<K: Clone + Hash + Eq, V: Clone> Inner<K, V> {
    fn do_prune_expired(&self) -> usize {
        let mut cache = self.cache.lock();
        let mut keys_to_remove = vec![];
        let now = Instant::now();
        for (k, entry) in cache.iter() {
            if now >= entry.expiration {
                keys_to_remove.push(k.clone());
            }
        }

        let mut pruned = 0;
        for k in keys_to_remove {
            if cache.remove(&k).is_some() {
                pruned += 1;
            }
        }
        pruned
    }
}

impl<K: Clone + Hash + Eq, V: Clone> CachePurger for Inner<K, V> {
    fn name(&self) -> &str {
        &self.name
    }
    fn purge(&self) -> usize {
        let mut cache = self.cache.lock();
        let num_entries = cache.len();
        cache.clear();
        num_entries
    }
    fn prune_expired(&self) -> usize {
        self.do_prune_expired()
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

        for purger in purgers {
            let name = purger.name();
            let num_entries = purger.prune_expired();
            tracing::trace!("expired {num_entries} entries from cache {name}");
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
struct Item<V> {
    item: V,
    expiration: Instant,
}

pub struct LruCacheWithTtl<K: Clone + Hash + Eq, V: Clone> {
    inner: Arc<Inner<K, V>>,
}

impl<K: Clone + Hash + Eq + Send + 'static, V: Clone + Send + 'static> LruCacheWithTtl<K, V> {
    #[deprecated = "use new_named instead"]
    pub fn new(capacity: usize) -> Self {
        Self::new_named("<anonymous>", capacity)
    }

    pub fn new_named<S: Into<String>>(name: S, capacity: usize) -> Self {
        let inner = Arc::new(Inner {
            name: name.into(),
            cache: Mutex::new(LruCache::new(capacity)),
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
        let mut cache = self.inner.cache.lock();
        let num_entries = cache.len();
        cache.clear();
        num_entries
    }

    pub fn get_with_expiry<Q: ?Sized>(&self, name: &Q) -> Option<(V, Instant)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let mut cache = self.inner.cache.lock();
        let entry = cache.get_mut(name)?;
        if Instant::now() < entry.expiration {
            Some((entry.item.clone(), entry.expiration))
        } else {
            cache.remove(name);
            None
        }
    }

    pub fn get<Q: ?Sized>(&self, name: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let mut cache = self.inner.cache.lock();
        let entry = cache.get_mut(name)?;
        if Instant::now() < entry.expiration {
            entry.item.clone().into()
        } else {
            cache.remove(name);
            None
        }
    }

    pub fn insert(&self, name: K, item: V, expiration: Instant) -> V {
        self.inner.cache.lock().insert(
            name,
            Item {
                item: item.clone(),
                expiration,
            },
        );
        item
    }

    pub fn prune_expired(&self) -> usize {
        self.inner.do_prune_expired()
    }

    /// Get an existing item, but if that item doesn't already exist,
    /// call `func` to provide a value that will be inserted and then
    /// returned.  This is done atomically wrt. other callers.
    pub fn get_or_insert<F: FnOnce() -> V>(&self, name: K, ttl: Duration, func: F) -> V {
        let mut cache = self.inner.cache.lock();
        if let Some(entry) = cache.get_mut(&name) {
            if Instant::now() < entry.expiration {
                return entry.item.clone();
            }
        }
        let item = func();
        cache.insert(
            name,
            Item {
                item: item.clone(),
                expiration: Instant::now() + ttl,
            },
        );
        item
    }
}
