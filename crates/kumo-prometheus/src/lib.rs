pub use crate::counter::*;
use crate::labels::MetricLabel;
use crate::registry::StreamingCollector;
use async_stream::stream;
use futures::stream::BoxStream;
use futures::StreamExt;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
pub use pastey as paste;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

mod counter;
pub mod counter_bundle;

#[macro_use]
pub mod labels;
pub mod parser;
pub mod registry;

struct CounterRegistryInner<K, V: AtomicCounterEntry> {
    map: RwLock<HashMap<K, V>>,
    name: &'static str,
    help: &'static str,
    is_gauge: bool,
}

/// Keep up to 4k at a time of pending text or json data
/// when streaming out the serialized counter values
const CHUNK_SIZE: usize = 4 * 1024;

impl<K: Clone + MetricLabel + Send + Sync, V: AtomicCounterEntry> StreamingCollector
    for CounterRegistryInner<K, V>
{
    fn stream_text(&'_ self, prefix: &Option<String>) -> BoxStream<'_, String> {
        /*
        # HELP tokio_total_overflow_count The number of times worker threads saturated their local queues.
        # TYPE tokio_total_overflow_count counter
        tokio_total_overflow_count 0
        total_connection_count{service="smtp_client:source2->loopback.dummy-mx.wezfurlong.org@smtp_client"} 25
        */

        let mut buffer = String::with_capacity(CHUNK_SIZE);
        buffer.push_str("# HELP ");
        let prefix = prefix.as_deref().unwrap_or("").to_string();
        buffer.push_str(&prefix);
        buffer.push_str(self.name);
        buffer.push(' ');
        buffer.push_str(self.help);
        buffer.push_str("\n# TYPE ");
        buffer.push_str(&prefix);
        buffer.push_str(self.name);
        buffer.push(' ');
        buffer.push_str(if self.is_gauge { "gauge" } else { "counter" });
        buffer.push('\n');

        let mut buffer = Some(buffer);

        let counters = {
            let map = self.map.read();
            let mut pairs = Vec::with_capacity(map.len());
            for (key, weak) in map.iter() {
                if let Some(strong) = weak.resolve() {
                    pairs.push((key.clone(), strong));
                }
            }
            pairs
        };

        stream! {
            for (key, counter) in counters {
                let Some(buf) = buffer.as_mut() else {break;};

                buf.push_str(&prefix);
                buf.push_str(self.name);
                key.emit_text_value(buf, &counter.get().to_string());
                buf.push('\n');

                let need_flush = buf.len() >= CHUNK_SIZE;

                if need_flush {
                    yield buffer.take().expect("always have buffer");
                    buffer.replace(String::with_capacity(CHUNK_SIZE));
                }
            }

            if let Some(buf) = buffer.take() {
                if !buf.is_empty(){
                    yield buf;
                }
            }

        }
        .boxed()
    }

    fn stream_json(&'_ self) -> BoxStream<'_, String> {
        let mut target = String::with_capacity(CHUNK_SIZE);
        target.push_str(",\n\"");
        target.push_str(self.name);
        target.push_str("\":{");
        if !self.help.is_empty() {
            target.push_str("\"help\":\"");
            target.push_str(self.help);
            target.push_str("\",");
        }
        target.push_str("\"type\":\"");
        target.push_str(if self.is_gauge { "gauge" } else { "counter" });
        target.push_str("\",\"value\":");

        let counters = {
            let map = self.map.read();
            let mut pairs = Vec::with_capacity(map.len());
            for (key, weak) in map.iter() {
                if let Some(strong) = weak.resolve() {
                    pairs.push((key.clone(), strong));
                }
            }
            pairs
        };

        stream! {
            if counters.is_empty() {
                target.push_str("null}");
                yield target;
                return;
            }

            let labels = K::label_names();

            if labels.len() == 1 {
                target.push_str("{\"");
                target.push_str(labels[0]);
                target.push_str("\":{");
            } else {
                target.push('[');
            }

            let mut buffer = Some(target);

            for (i, (key, counter)) in counters.iter().enumerate() {
                let Some(target) = buffer.as_mut() else {break;};
                if i > 0 {
                    target.push_str(",\n");
                }

                let value = counter.get().to_string();
                key.emit_json_value(target,&value);

                let need_flush = target.len() >= CHUNK_SIZE;

                if need_flush {
                    yield buffer.take().expect("always have buffer");
                    buffer.replace(String::with_capacity(CHUNK_SIZE));
                }
            }

            let Some(mut target) = buffer.take() else {return;};
            if labels.len() == 1 {
                target.push_str("}}}");
            } else {
                target.push_str("]}");
            }

            yield target;
        }
        .boxed()
    }

    fn prune(&self) {
        if !V::needs_pruning() {
            return;
        }

        let mut map = self.map.write();
        map.retain(|_key, entry| entry.resolve().is_some());
    }
}

/// Either a Counter or Gauge with a specific name, where there can
/// be multiple labelled counter instances.
///
/// CounterRegistry has a PruningCounterRegistry variant which will
/// drop unreferenced counter instances when they fall out of scope.
///
/// The key type K must be created via the label_key! macro provided
/// by this crate. It allows making type-safe keys and resolving
/// counter instances without making extraneous copies of the keys.
///
/// CounterRegistry implements the StreamingCollector trait which
/// allows for efficient streaming serialization of its set of
/// counters in either text or json format.
pub struct CounterRegistry<K, V: AtomicCounterEntry = AtomicCounter> {
    inner: Arc<CounterRegistryInner<K, V>>,
}

pub type PruningCounterRegistry<K> = CounterRegistry<K, WeakAtomicCounter>;

impl<K, V: AtomicCounterEntry> Clone for CounterRegistry<K, V> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<K: Clone + Send + Sync + MetricLabel + 'static, V: AtomicCounterEntry + 'static>
    CounterRegistry<K, V>
{
    /// Register a set of Counters, values that are only allowed
    /// to increment.
    pub fn register(name: &'static str, help: &'static str) -> Self {
        Self::register_impl(name, help, false)
    }

    /// Register a set of Gauges, values that are allowed to increase and decrease.
    pub fn register_gauge(name: &'static str, help: &'static str) -> Self {
        Self::register_impl(name, help, true)
    }

    fn register_impl(name: &'static str, help: &'static str, is_gauge: bool) -> Self {
        let me = Self {
            inner: Arc::new(CounterRegistryInner {
                map: Default::default(),
                name,
                help,
                is_gauge,
            }),
        };

        crate::registry::Registry::register(me.inner.clone());

        me
    }
}

impl<K, V> CounterRegistry<K, V>
where
    V: AtomicCounterEntry,
    K: Eq + Hash + MetricLabel,
{
    /// Resolve an already-existing counter for the given key, or None
    /// if there either has never been such a value, or if it was pruned.
    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<AtomicCounter>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let map = self.inner.map.read();
        map.get(key).and_then(|weak| weak.resolve())
    }

    /// Resolve an already-existing counter for the given key, creating
    /// a new one if it didn't already exist, or was previously pruned.
    pub fn get_or_create<'a, Q: ?Sized>(&self, key: &'a Q) -> AtomicCounter
    where
        K: Borrow<Q> + From<&'a Q>,
        Q: Hash + Eq,
    {
        let map = self.inner.map.upgradable_read();
        if let Some(weak) = map.get(key) {
            if let Some(strong) = weak.resolve() {
                return strong;
            }
        }

        let mut map = RwLockUpgradableReadGuard::upgrade(map);

        // Check again, as we may have lost a race
        if let Some(weak) = map.get(key) {
            if let Some(strong) = weak.resolve() {
                return strong;
            }
        }

        let result = AtomicCounter::new();
        map.insert(key.into(), V::make_storable(&result));

        result
    }
}
