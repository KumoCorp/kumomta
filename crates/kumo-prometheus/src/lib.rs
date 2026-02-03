pub use crate::counter::*;
use crate::labels::MetricLabel;
use crate::registry::StreamingCollector;
use async_stream::stream;
use futures::stream::BoxStream;
use futures::StreamExt;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
pub use pastey as paste;
use serde::Serialize;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, LazyLock};

mod counter;
pub mod counter_bundle;

#[macro_use]
pub mod labels;
pub mod parser;
pub mod registry;
pub use prometheus;

struct CounterRegistryInner<K, V: AtomicCounterEntry> {
    map: RwLock<HashMap<K, V>>,
    name: &'static str,
    help: String,
    metric_type: MetricType,
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
        buffer.push_str(&self.help);
        buffer.push_str("\n# TYPE ");
        buffer.push_str(&prefix);
        buffer.push_str(self.name);
        buffer.push(' ');
        buffer.push_str(self.metric_type.label());
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
            target.push_str(&self.help);
            target.push_str("\",");
        }
        target.push_str("\"type\":\"");
        target.push_str(self.metric_type.label());
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

impl<K, V: AtomicCounterEntry> CounterRegistry<K, V> {
    pub fn metric_type(&self) -> MetricType {
        self.inner.metric_type
    }
}

pub type PruningCounterRegistry<K> = CounterRegistry<K, WeakAtomicCounter>;

#[derive(Serialize, Clone, Copy)]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
}

impl MetricType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

#[derive(Serialize, Clone, Copy)]
pub enum MetricPrune {
    Pruning,
    NonPruning,
}

#[derive(Serialize, Clone)]
pub struct CounterDescription {
    /// The name of the counter, as it appears in the metric export
    pub name: String,
    /// one-line help description that is included in the metric export
    pub help: String,
    /// If multi-line comments are present, this will hold the comments
    /// after the first help line.
    pub doc: Option<String>,
    /// What sort of metric this is
    pub metric_type: MetricType,
    /// If the metric has labels, this lists them out
    pub label_names: Vec<String>,
    /// If the metric is a histogram, this holds the bucket thresholds
    pub buckets: Vec<f64>,
    /// True if the metric is subject to pruning
    pub pruning: MetricPrune,
}

/// Accumulates metric metadata
#[linkme::distributed_slice]
pub static COUNTER_METADATA: [fn() -> CounterDescription];

fn compute_metadata() -> Vec<CounterDescription> {
    let mut metadata = vec![];

    for func in COUNTER_METADATA.iter() {
        let desc = (func)();
        metadata.push(desc);
    }

    metadata.sort_by(|a, b| a.name.cmp(&b.name));
    metadata
}

static METADATA: LazyLock<Vec<CounterDescription>> = LazyLock::new(compute_metadata);

pub fn export_metadata() -> Vec<CounterDescription> {
    // We're generally called early in a process lifecycle, and Registry
    // requires a running tokio environment otherwise it will panic,
    // so we make a little one just for this call.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("failed to make single thread runtime for export_metadata call");
    let _guard = rt.enter();
    METADATA.clone()
}

/// This macro matches a series of doc comment attributes.
/// Multi-line doc comments appear as a sequence of doc
/// comment attributes, so we need to be able to match
/// both the individual case and the sequence, and map
/// them back to a single string.
///
/// While this macro accepts the no-doc-comment case,
/// we require that every metric have a doc comment,
/// so we will emit a compile error if none were present.
#[macro_export]
macro_rules! mandatory_doc {
    ($doc:expr) => {
        $doc
    };
    ($($doc:expr)+) => {
        // Join the sequence into a multi-line string
        concat!($($doc, "\n",)+)
    };
    () => {
        compile_error!("doc comments are mandatory")
    };
}

/// Utility function for dealing with doc comment metadata.
/// Look for two successive line breaks; if they are present
/// they denote the break between the short first-logical-line
/// and a longer descriptive exposition.  Returns that first
/// logical line and the optional exposition.
pub fn split_help(help: &str) -> (String, Option<&str>) {
    fn one_line(s: &str) -> String {
        s.replace("\n", " ").trim().to_string()
    }

    match help.split_once("\n\n") {
        Some((a, b)) => (one_line(a), Some(b)),
        None => (one_line(help), None),
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __histogram_buckets {
    () => {
        $crate::prometheus::DEFAULT_BUCKETS.to_vec()
    };
    ($buckets:expr) => {
        $buckets
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __register_metric {
    (
        $sym:ident,
        $name:expr,
        $($doc:expr)+,
        $metric:expr,
        $labels:expr,
        $pruning:expr
    ) => {
        $crate::__register_metric!($sym, $name, $($doc)+, $metric, $labels, $pruning, vec![]);
    };

    (
        $sym:ident,
        $name:expr,
        $($doc:expr)+,
        $metric:expr,
        $labels:expr,
        $pruning:expr,
        $buckets:expr
    ) => {
        // Link into COUNTER_METADATA
        $crate::paste::paste! {
            #[linkme::distributed_slice($crate::COUNTER_METADATA)]
            static [<VIVIFY_ $sym>]: fn() -> $crate::CounterDescription = || {
                ::std::sync::LazyLock::force(&$sym);
                let (help, doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));
                let labels : &[&str] = $labels;
                $crate::CounterDescription {
                    name: $name.to_string(),
                    help: help.to_string(),
                    doc: doc.map(|s| s.to_string()),
                    metric_type: $metric,
                    label_names: labels.iter().map(|s| s.to_string()).collect(),
                    buckets: $buckets,
                    pruning: $pruning,
                }
            };
        }
    }
}

/// This macro aids in declaring metrics.  Usage looks like:
///
/// ```rust
/// declare_metric! {
/// /// The number of active outgoing connections in the system,
/// /// keyed by the service name.
/// pub static CONN_GAUGE: PruningGaugeRegistry<ServiceKey>("connection_count");
/// }
/// ```
///
/// This will wrap the declaration of the global into a LazyLock as
/// well as capture metadata about the metric in a way that allows
/// it to be retrieved via the `export_metadata()` function.
///
/// Doc comments are required for every metric declared by this
/// macro.
///
/// Different types of metric collector are supported, not just
/// the `PruningGaugeRegistry` shown above.
///
/// `PruningGaugeRegistry` is not actually a real type, it is
/// some sugar allowed here to enable setting up a Gauge
/// rather than a Counter.
#[macro_export]
macro_rules! declare_metric {
    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        CounterRegistry<$key:ty>(
            $name:expr $(,)?
        );
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::CounterRegistry<$key>> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));
                    $crate::CounterRegistry::register($name, help)
                });

        $crate::__register_metric!($sym, $name, $($doc)*, $crate::MetricType::Counter,
            <$key as $crate::labels::MetricLabel>::label_names(),
            $crate::MetricPrune::NonPruning
        );
    };

    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        PruningCounterRegistry<$key:ty>(
            $name:expr $(,)?
        );
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::PruningCounterRegistry<$key>> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));
                    $crate::PruningCounterRegistry::register($name, help)
                });

        $crate::__register_metric!($sym, $name, $($doc)*, $crate::MetricType::Counter,
            <$key as $crate::labels::MetricLabel>::label_names(),
            $crate::MetricPrune::Pruning
        );
    };

    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        PruningGaugeRegistry<$key:ty>(
            $name:expr $(,)?
        );
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::PruningCounterRegistry<$key>> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));
                    $crate::PruningCounterRegistry::register_gauge($name, help)
                });

        $crate::__register_metric!($sym, $name, $($doc)*, $crate::MetricType::Gauge,
            <$key as $crate::labels::MetricLabel>::label_names(),
            $crate::MetricPrune::Pruning
        );
    };


    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        IntGaugeVec(
            $name:expr,
            $labels:expr $(,)*
        );
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::prometheus::IntGaugeVec> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));

                    $crate::prometheus::register_int_gauge_vec!(
                        $name,
                        help,
                        $labels
                    ).unwrap()
                });

        $crate::__register_metric!($sym, $name, $($doc)*, $crate::MetricType::Gauge,
            $labels, $crate::MetricPrune::NonPruning);
    };
    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        IntCounterVec(
            $name:expr,
            $labels:expr $(,)*
        );
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::prometheus::IntCounterVec> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));

                    $crate::prometheus::register_int_counter_vec!(
                        $name,
                        help,
                        $labels
                    ).unwrap()
                });

        $crate::__register_metric!($sym, $name, $($doc)*, $crate::MetricType::Counter,
            &$labels[..], $crate::MetricPrune::NonPruning);
    };
    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        CounterVec(
            $name:expr,
            $labels:expr $(,)*
        );
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::prometheus::CounterVec> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));

                    $crate::prometheus::register_counter_vec!(
                        $name,
                        help,
                        $labels
                    ).unwrap()
                });

        $crate::__register_metric!($sym, $name, $($doc)*, $crate::MetricType::Counter,
            &$labels[..], $crate::MetricPrune::NonPruning);
    };
    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        HistogramVec(
            $name:expr,
            $labels:expr
            $(, $buckets:expr)?
            $(,)*
        );
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::prometheus::HistogramVec> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));

                    $crate::prometheus::register_histogram_vec!(
                        $name,
                        help,
                        $labels
                        $(,$buckets)?
                    ).unwrap()
                });

        $crate::__register_metric!($sym, $name, $($doc)*, $crate::MetricType::Histogram,
            &$labels[..],
            $crate::MetricPrune::NonPruning,
            $crate::__histogram_buckets!($($buckets)?)
        );
    };

    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        Histogram($name:expr $(, $buckets:expr)?);
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::prometheus::Histogram> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));

                    $crate::prometheus::register_histogram!(
                        $name,
                        help,
                        $($buckets)?
                    ).unwrap()
                });

        $crate::__register_metric!($sym, $name, $($doc)*, $crate::MetricType::Histogram, &[],
            $crate::MetricPrune::NonPruning,
            $crate::__histogram_buckets!($($buckets)?)
        );
    };

    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        IntCounter($name:expr);
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::prometheus::IntCounter> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));

                    $crate::prometheus::register_int_counter!(
                        $name,
                        help,
                    ).unwrap()
                });

        $crate::__register_metric!($sym, $name, $($doc)*,
            $crate::MetricType::Counter, &[], $crate::MetricPrune::NonPruning);
    };

    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        IntGauge($name:expr);
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::prometheus::IntGauge> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));

                    $crate::prometheus::register_int_gauge!(
                        $name,
                        help,
                    ).unwrap()
                });

        $crate::__register_metric!($sym, $name, $($doc)*,
            $crate::MetricType::Gauge, &[], $crate::MetricPrune::NonPruning);
    };
    (
        $(#[doc = $doc:expr])*
        $vis:vis
        static $sym:ident:
        Gauge($name:expr);
    ) => {
        $(#[doc = $doc])*
        $vis static $sym: ::std::sync::LazyLock<$crate::prometheus::Gauge> =
            ::std::sync::LazyLock::new(
                || {
                    let (help, _doc) = $crate::split_help($crate::mandatory_doc!($($doc)*));

                    $crate::prometheus::register_gauge!(
                        $name,
                        help,
                    ).unwrap()
                });

        $crate::__register_metric!($sym, $name, $($doc)*,
            $crate::MetricType::Gauge, &[], $crate::MetricPrune::NonPruning);
    };
}

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
    pub fn register(name: &'static str, help: String) -> Self {
        Self::register_impl(name, help, MetricType::Counter)
    }

    /// Register a set of Gauges, values that are allowed to increase and decrease.
    pub fn register_gauge(name: &'static str, help: String) -> Self {
        Self::register_impl(name, help, MetricType::Gauge)
    }

    fn register_impl(name: &'static str, help: String, metric_type: MetricType) -> Self {
        let me = Self {
            inner: Arc::new(CounterRegistryInner {
                map: Default::default(),
                name,
                help,
                metric_type,
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
