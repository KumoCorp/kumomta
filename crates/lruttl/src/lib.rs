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

use lru_cache::LruCache;
use parking_lot::Mutex;
use std::borrow::Borrow;
use std::hash::Hash;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct Item<V> {
    item: V,
    expiration: Instant,
}

pub struct LruCacheWithTtl<K: Hash + Eq, V: Clone> {
    cache: Mutex<LruCache<K, Item<V>>>,
}

impl<K: Hash + Eq, V: Clone> LruCacheWithTtl<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: Mutex::new(LruCache::new(capacity)),
        }
    }

    pub fn get_with_expiry<Q: ?Sized>(&self, name: &Q) -> Option<(V, Instant)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let mut cache = self.cache.lock();
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
        let mut cache = self.cache.lock();
        let entry = cache.get_mut(name)?;
        if Instant::now() < entry.expiration {
            entry.item.clone().into()
        } else {
            cache.remove(name);
            None
        }
    }

    pub fn insert(&self, name: K, item: V, expiration: Instant) -> V {
        self.cache.lock().insert(
            name,
            Item {
                item: item.clone(),
                expiration,
            },
        );
        item
    }

    /// Get an existing item, but if that item doesn't already exist,
    /// call `func` to provide a value that will be inserted and then
    /// returned.  This is done atomically wrt. other callers.
    pub fn get_or_insert<F: FnOnce() -> V>(&self, name: K, ttl: Duration, func: F) -> V {
        let mut cache = self.cache.lock();
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
