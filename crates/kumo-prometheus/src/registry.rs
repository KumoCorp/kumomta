use async_stream::stream;
use futures::prelude::*;
use futures::stream::BoxStream;
use parking_lot::Mutex;
use prometheus::proto::{Metric, MetricFamily};
use std::sync::{Arc, LazyLock};

pub trait StreamingCollector {
    /// Stream chunks of text in prometheus text exposition format
    fn stream_text(&'_ self, prefix: &Option<String>) -> BoxStream<'_, String>;
    /// Stream chunks in our json format, as chunks of text
    fn stream_json(&'_ self) -> BoxStream<'_, String>;
    /// Prune any stale entries from this collector
    fn prune(&self);
}

/// Keeps track of all streaming collector instances
pub struct Registry {
    collectors: Mutex<Arc<Vec<Arc<dyn StreamingCollector + Send + Sync>>>>,
}

impl Registry {
    /// Get the Registry singleton, and spawn the pruning task if it
    /// hasn't already been launched.
    pub fn get() -> &'static Self {
        static REG: LazyLock<Registry> = LazyLock::new(|| {
            tokio::spawn(Registry::pruner());
            Registry {
                collectors: Mutex::new(Arc::new(vec![])),
            }
        });
        &REG
    }

    /// Periodically maintain the hashmaps, removing any pruned entries
    async fn pruner() {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            let collectors = Self::get_collectors();
            for c in collectors.iter() {
                c.prune();
            }
        }
    }

    /// Register a new collector
    pub fn register(collector: Arc<dyn StreamingCollector + Send + Sync>) {
        let reg = Self::get();
        let mut collectors = reg.collectors.lock();
        let mut new_set: Vec<_> = collectors.iter().map(Arc::clone).collect();
        new_set.push(collector);
        *collectors = Arc::new(new_set);
    }

    fn get_collectors() -> Arc<Vec<Arc<dyn StreamingCollector + Send + Sync>>> {
        Self::get().collectors.lock().clone()
    }

    /// Produce a stream of String chunks that represent all known metrics
    /// in the Prometheus exposition format.
    ///
    /// This will include the MetricFamily's that have been registered with
    /// the prometheus crate and then supplement the output with our own
    /// set of registered streaming collectors.
    ///
    /// The optional prefix parameter is used to "namespace" the returned
    /// metric names.
    pub fn stream_text(prefix: Option<String>) -> BoxStream<'static, String> {
        let collectors = Self::get_collectors();

        stream! {
            let mut metrics = prometheus::default_registry().gather();
            if let Some(prefix) = &prefix {
                metrics.iter_mut().for_each(|metric| {
                    let name = format!("{prefix}{}", metric.get_name());
                    metric.set_name(name);
                });
            }
            if let Ok(report) = prometheus::TextEncoder::new().encode_to_string(&metrics) {
                yield report;
            }

            for c in collectors.iter() {
                let mut text_stream = c.stream_text(&prefix);
                while let Some(chunk) = text_stream.next().await {
                    yield chunk;
                }
            }
        }
        .boxed()
    }

    /// Produce a stream of String chunks that represent all known metrics
    /// in the informal kumomta json format.
    ///
    /// This will include the MetricFamily's that have been registered with
    /// the prometheus crate and then supplement the output with our own
    /// set of registered streaming collectors.
    pub fn stream_json() -> BoxStream<'static, String> {
        let collectors = Self::get_collectors();

        stream! {
            let mut buf = "{".to_string();
            let metrics = prometheus::default_registry().gather();
            metrics_to_partial_json(&metrics, &mut buf);
            yield buf;

            for c in collectors.iter() {
                let mut text_stream = c.stream_json();
                while let Some(chunk) = text_stream.next().await {
                    yield chunk;
                }
            }

            yield "}".to_string();
        }
        .boxed()
    }
}

/// This function emits prometheus crate MetricFamily's into the informal
/// kumomta json representation.  The json text fragment is emitted into
/// the provided target String.
/// The result is not a complete JSON document, it is just the keys and values
/// produced from the provided list of metrics.
fn metrics_to_partial_json(metrics: &[MetricFamily], target: &mut String) {
    use prometheus::proto::MetricType;

    for (midx, mf) in metrics.iter().enumerate() {
        if midx > 0 {
            target.push(',');
        }
        let name = mf.get_name();
        let help = mf.get_help();

        target.push('"');
        target.push_str(name);
        target.push_str("\":{");
        if !help.is_empty() {
            target.push_str("\"help\":\"");
            target.push_str(help);
            target.push_str("\",");
        }

        let metric_type = mf.get_field_type();

        target.push_str("\"type\":\"");
        target.push_str(&format!("{metric_type:?}").to_lowercase());
        target.push_str("\",\"value\":");

        let metric_values = mf.get_metric();
        if metric_values.is_empty() {
            target.push_str("null}");
            continue;
        }

        let first_label = metric_values[0].get_label();
        if first_label.len() == 1 {
            target.push_str("{\"");
            target.push_str(first_label[0].get_name());
            target.push_str("\":{");
        } else if first_label.len() > 1 {
            target.push('[');
        }

        for (i, metric) in metric_values.iter().enumerate() {
            let label = metric.get_label();

            if i > 0 {
                target.push(',');
            }

            fn emit_value(metric_type: MetricType, metric: &Metric, target: &mut String) {
                match metric_type {
                    MetricType::COUNTER | MetricType::GAUGE => {
                        let value = if metric_type == MetricType::COUNTER {
                            metric.get_counter().get_value()
                        } else {
                            metric.get_gauge().get_value()
                        };
                        target.push_str(&value.to_string());
                    }
                    MetricType::HISTOGRAM => {
                        let hist = metric.get_histogram();

                        let count = hist.get_sample_count();
                        let sum = hist.get_sample_sum();
                        let avg = if count != 0 { sum / count as f64 } else { 0. };

                        let mut bucket = vec![];
                        for b in hist.get_bucket() {
                            bucket.push(vec![b.get_upper_bound(), b.get_cumulative_count() as f64]);
                        }

                        let hist_value = serde_json::json!({
                            "count": count,
                            "sum": sum,
                            "avg": avg,
                            "bucket": bucket,
                        });

                        if let Ok(s) = serde_json::to_string(&hist_value) {
                            target.push_str(&s);
                        }
                    }
                    _ => {
                        // Other types are currently not implemented
                        // as we don't currently export any other type
                        target.push_str("null");
                    }
                }
            }

            if label.is_empty() {
                emit_value(metric_type, metric, target);
                break;
            }

            if label.len() == 1 {
                target.push('"');
                target.push_str(label[0].get_value());
                target.push_str("\":");
                emit_value(metric_type, metric, target);
                continue;
            }

            target.push('{');
            for pair in label {
                target.push('"');
                target.push_str(pair.get_name());
                target.push_str("\":\"");
                target.push_str(pair.get_value());
                target.push_str("\",");
            }
            target.push_str("\"@\":");
            emit_value(metric_type, metric, target);
            target.push('}');
        }

        if first_label.len() == 1 {
            target.push_str("}}}");
        } else if first_label.len() > 1 {
            target.push_str("]}");
        } else {
            target.push('}');
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use prometheus::proto::{Counter, LabelPair, Metric, MetricFamily, MetricType};

    #[test]
    fn test_json_encode_counter_no_help() {
        let mut family = MetricFamily::new();
        family.set_name("family_name".into());
        family.set_field_type(MetricType::COUNTER);

        let mut metric = Metric::new();
        metric.set_counter(Counter::new());
        family.set_metric(vec![metric].into());

        let mut buf = "{".into();
        metrics_to_partial_json(&[family], &mut buf);
        buf.push('}');

        println!("{buf}");
        let value: serde_json::Value = serde_json::from_str(&buf).unwrap();
        println!("{value:?}");
        assert_eq!(
            value,
            serde_json::json!({
                "family_name": {
                    "type": "counter",
                    "value": 0
                }
            })
        );
    }

    #[test]
    fn test_json_encode_histogram_one_label() {
        use prometheus::core::Collector;
        let hist = prometheus::Histogram::with_opts(prometheus::HistogramOpts::new(
            "hist_name",
            "hist_help",
        ))
        .unwrap();
        let family = hist.collect();

        let mut buf = "{".into();
        metrics_to_partial_json(&family, &mut buf);
        buf.push('}');

        println!("{buf}");
        let value: serde_json::Value = serde_json::from_str(&buf).unwrap();
        println!("{value:?}");
        assert_eq!(
            value,
            serde_json::json!({
            "hist_name": {
                "help": "hist_help",
                "type": "histogram",
                "value": {
                    "avg": 0.0,
                    "bucket": [
                        [0.005, 0.0],
                        [0.01, 0.0],
                        [0.025, 0.0],
                        [0.05, 0.0],
                        [0.1, 0.0],
                        [0.25, 0.0],
                        [0.5, 0.0],
                        [1.0, 0.0],
                        [2.5, 0.0],
                        [5.0, 0.0],
                        [10.0, 0.0]
                    ],
                    "count":0,
                    "sum":0.0
                }}})
        );
    }

    #[test]
    fn test_json_encode_counter_one_label() {
        let mut family = MetricFamily::new();
        family.set_name("family_name".into());
        family.set_field_type(MetricType::COUNTER);

        let mut metric = Metric::new();
        metric.set_counter(Counter::new());
        let mut label = LabelPair::new();
        label.set_name("label_name".into());
        label.set_value("label_value".into());
        metric.set_label(vec![label].into());
        family.set_metric(vec![metric].into());

        let mut buf = "{".into();
        metrics_to_partial_json(&[family], &mut buf);
        buf.push('}');

        println!("{buf}");
        let value: serde_json::Value = serde_json::from_str(&buf).unwrap();
        println!("{value:?}");
        assert_eq!(
            value,
            serde_json::json!({
                "family_name": {
                    "type": "counter",
                    "value": {
                        "label_name": {
                            "label_value": 0
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn test_json_encode_counter_one_label_two_values() {
        let mut family = MetricFamily::new();
        family.set_name("family_name".into());
        family.set_field_type(MetricType::COUNTER);

        let mut metric = Metric::new();
        metric.set_counter(Counter::new());
        let mut label = LabelPair::new();
        label.set_name("label_name".into());
        label.set_value("label_value".into());
        metric.set_label(vec![label].into());

        let mut metric2 = Metric::new();
        metric2.set_counter(Counter::new());
        let mut label2 = LabelPair::new();
        label2.set_name("label_name".into());
        label2.set_value("2nd".into());
        metric2.set_label(vec![label2].into());

        family.set_metric(vec![metric, metric2].into());

        let mut buf = "{".into();
        metrics_to_partial_json(&[family], &mut buf);
        buf.push('}');

        println!("{buf}");
        let value: serde_json::Value = serde_json::from_str(&buf).unwrap();
        println!("{value:?}");
        assert_eq!(
            value,
            serde_json::json!({
                "family_name": {
                    "type": "counter",
                    "value": {
                        "label_name": {
                            "label_value": 0,
                            "2nd": 0
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn test_json_encode_counter_two_labels() {
        let mut family = MetricFamily::new();
        family.set_name("family_name".into());
        family.set_field_type(MetricType::COUNTER);

        let mut metric = Metric::new();
        metric.set_counter(Counter::new());
        let mut label1 = LabelPair::new();
        label1.set_name("first_label_name".into());
        label1.set_value("first_label_value".into());
        let mut label2 = LabelPair::new();
        label2.set_name("2nd_label_name".into());
        label2.set_value("2nd_label_value".into());
        metric.set_label(vec![label1, label2].into());
        family.set_metric(vec![metric].into());

        let mut buf = "{".into();
        metrics_to_partial_json(&[family], &mut buf);
        buf.push('}');

        println!("{buf}");
        let value: serde_json::Value = serde_json::from_str(&buf).unwrap();
        println!("{value:?}");
        assert_eq!(
            value,
            serde_json::json!({
                "family_name": {
                    "type": "counter",
                    "value": [{
                        "first_label_name": "first_label_value",
                        "2nd_label_name": "2nd_label_value",
                        "@": 0
                        }
                    ]
                }
            })
        );
    }

    #[test]
    fn test_json_encode_counter_with_help() {
        let mut family = MetricFamily::new();
        family.set_name("family_name".into());
        family.set_help("me".into());
        family.set_field_type(MetricType::COUNTER);

        let mut metric = Metric::new();
        metric.set_counter(Counter::new());
        family.set_metric(vec![metric].into());

        let mut buf = "{".into();
        metrics_to_partial_json(&[family], &mut buf);
        buf.push('}');

        println!("{buf}");
        let value: serde_json::Value = serde_json::from_str(&buf).unwrap();
        println!("{value:?}");
        assert_eq!(
            value,
            serde_json::json!({
                "family_name": {
                    "type": "counter",
                    "help": "me",
                    "value": 0
                }
            })
        );
    }
}
