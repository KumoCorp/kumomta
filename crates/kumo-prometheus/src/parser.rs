use map_vec::Map;
use memchr::memchr_iter;
use std::borrow::Borrow;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Default, Eq, PartialEq, Clone, Copy)]
enum MetricType {
    #[default]
    Unknown,
    Counter,
    Gauge,
    Histogram,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct InternString(Arc<String>);

impl std::ops::Deref for InternString {
    type Target = str;
    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl PartialEq<&str> for InternString {
    fn eq(&self, other: &&str) -> bool {
        self.0.as_str() == *other
    }
}

impl std::fmt::Display for InternString {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.as_str().fmt(fmt)
    }
}

impl std::fmt::Debug for InternString {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.as_str().fmt(fmt)
    }
}

impl Borrow<str> for InternString {
    fn borrow(&self) -> &str {
        self.0.as_str()
    }
}

impl InternString {
    pub fn new(s: &str) -> Self {
        Self(Arc::new(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for InternString {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

pub struct Parser {
    strings: HashSet<InternString>,
    buffer: Vec<u8>,
    current_type: MetricType,
    histogram: Option<HistogramMetric>,
}

impl Parser {
    pub fn new() -> Self {
        Parser {
            strings: HashSet::new(),
            buffer: vec![],
            current_type: MetricType::Unknown,
            histogram: None,
        }
    }

    fn intern_string(&mut self, s: &str) -> InternString {
        match self.strings.get(s) {
            Some(k) => k.clone(),
            None => {
                let v = InternString::new(s);
                self.strings.insert(v.clone());
                v
            }
        }
    }

    fn flush_histogram<F: FnMut(Metric)>(&mut self, func: &mut F) {
        if let Some(histogram) = self.histogram.take() {
            (func)(Metric::Histogram(histogram));
        }
    }

    pub fn push_bytes<F: FnMut(Metric), S: AsRef<[u8]>>(
        &mut self,
        data: S,
        is_final: bool,
        mut func: F,
    ) -> anyhow::Result<()> {
        let data = data.as_ref();

        if !self.buffer.is_empty() {
            if let Some(nl) = memchr::memchr(b'\n', data) {
                self.buffer.extend_from_slice(&data[0..=nl]);
                let buffer = std::mem::take(&mut self.buffer);
                self.push_bytes_sol(&buffer, false, &mut func)?;
                self.push_bytes_sol(&data[nl + 1..], is_final, &mut func)
            } else {
                self.buffer.extend_from_slice(data);
                Ok(())
            }
        } else {
            self.push_bytes_sol(data, is_final, &mut func)
        }
    }

    fn push_bytes_sol<F: FnMut(Metric)>(
        &mut self,
        buffer: &[u8],
        is_final: bool,
        func: &mut F,
    ) -> anyhow::Result<()> {
        let mut start_of_line = 0;
        for nl in memchr_iter(b'\n', buffer) {
            let line = &buffer[start_of_line..nl];
            start_of_line = nl + 1;
            if line.is_empty() {
                continue;
            }
            let line = std::str::from_utf8(line)?;

            if line.starts_with("# TYPE ") {
                self.flush_histogram(func);
                match line.rsplit(|b| b == ' ').next() {
                    Some("counter") => self.current_type = MetricType::Counter,
                    Some("gauge") => self.current_type = MetricType::Gauge,
                    Some("histogram") => self.current_type = MetricType::Histogram,
                    Some(unknown) => anyhow::bail!("unknown metric type '{unknown}'"),
                    None => anyhow::bail!("invalid TYPE line '{line}'"),
                }

                continue;
            }

            if line.starts_with("#") {
                continue;
            }

            let Some((name_info, value)) = line.rsplit_once(' ') else {
                anyhow::bail!("invalid line {line}");
            };
            let value = match value.parse::<f64>() {
                Ok(v) => v,
                Err(err) => match value {
                    "+Inf" => f64::INFINITY,
                    "-Inf" => f64::NEG_INFINITY,
                    _ => anyhow::bail!("Error parsing value from {line}: {err:#}"),
                },
            };

            let mut labels = Map::new();

            let name = if let Some((name, rest)) = name_info.split_once('{') {
                let Some(mut label_text) = rest.strip_suffix("}") else {
                    anyhow::bail!("invalid name in {line}");
                };

                while !label_text.is_empty() {
                    let Some((label_name, rest)) = label_text.split_once("=\"") else {
                        anyhow::bail!("invalid labels in {line}");
                    };

                    let Some((label_value, rest)) = rest.split_once("\"") else {
                        anyhow::bail!("invalid labels in {line}");
                    };

                    let rest = rest.strip_prefix(",").unwrap_or(rest);
                    let rest = rest.strip_prefix(" ").unwrap_or(rest);
                    label_text = rest;

                    labels.insert(
                        self.intern_string(label_name),
                        // There's no point interning the value, as the cardinality
                        // of data is too high and the hit rate for this is too low
                        // vs. the cost of interning. With 2,162,586 metrics the
                        // runtime of just parsing the data with this interned is
                        // ~1.8 seconds. Without it interning label_value it goes
                        // down to 0.35s
                        InternString::new(label_value),
                    );
                }

                self.intern_string(name)
            } else {
                self.intern_string(name_info)
            };

            match self.current_type {
                MetricType::Counter => {
                    (func)(Metric::Counter(CounterMetric {
                        name,
                        labels,
                        value,
                    }));
                }
                MetricType::Gauge => {
                    (func)(Metric::Gauge(GaugeMetric {
                        name,
                        labels,
                        value,
                    }));
                }
                MetricType::Histogram => {
                    let Some(hist_name) = name
                        .strip_suffix("_bucket")
                        .or_else(|| name.strip_suffix("_count"))
                        .or_else(|| name.strip_suffix("_sum"))
                    else {
                        anyhow::bail!("unexpected histogram counter name in {line}");
                    };

                    let labels_less_le = {
                        let mut l = labels.clone();
                        l.remove("le");
                        l
                    };

                    let need_flush = self
                        .histogram
                        .as_ref()
                        .map(|hist| hist.name != hist_name || hist.labels != labels_less_le)
                        .unwrap_or(true);
                    if need_flush {
                        self.flush_histogram(func);
                        let histogram = HistogramMetric {
                            name: self.intern_string(hist_name),
                            labels: labels_less_le.clone(),
                            sum: 0.,
                            count: 0.,
                            bucket: vec![],
                        };
                        self.histogram.replace(histogram);
                    }

                    let Some(hist) = self.histogram.as_mut() else {
                        anyhow::bail!("histogram isn't set? impossible!");
                    };

                    if name.ends_with("_bucket") {
                        let Some(le) = labels.get("le").and_then(|le| le.parse::<f64>().ok())
                        else {
                            anyhow::bail!("failed to parse le as float in {line}");
                        };
                        hist.bucket.push((le, value));
                    } else if name.ends_with("_count") {
                        hist.count = value;
                    } else if name.ends_with("_sum") {
                        hist.sum = value;
                    } else {
                        anyhow::bail!("unexpected histogram case {line}");
                    }
                }
                MetricType::Unknown => {
                    anyhow::bail!("unknown metric type for {name} {value}");
                }
            }
        }
        let remainder = &buffer[start_of_line..];
        if remainder.is_empty() {
            self.buffer.clear();
        } else {
            self.buffer = remainder.to_vec();
        }

        if is_final {
            self.flush_histogram(func);
        }

        if is_final && !self.buffer.is_empty() {
            anyhow::bail!(
                "final chunk received and we still have buffered data:\n{}",
                String::from_utf8_lossy(&self.buffer)
            );
        }

        Ok(())
    }

    pub fn parse<S: AsRef<[u8]>>(&mut self, data: S) -> anyhow::Result<Vec<Metric>> {
        let mut metrics = vec![];
        self.push_bytes(data, true, |metric| metrics.push(metric))?;
        Ok(metrics)
    }
}

#[derive(Debug, PartialEq)]
pub enum Metric {
    Counter(CounterMetric),
    Gauge(GaugeMetric),
    Histogram(HistogramMetric),
}

impl Metric {
    pub fn name(&self) -> &InternString {
        match self {
            Self::Counter(c) => &c.name,
            Self::Gauge(g) => &g.name,
            Self::Histogram(h) => &h.name,
        }
    }

    pub fn labels(&self) -> &Map<InternString, InternString> {
        match self {
            Self::Counter(c) => &c.labels,
            Self::Gauge(g) => &g.labels,
            Self::Histogram(h) => &h.labels,
        }
    }

    pub fn value(&self) -> f64 {
        match self {
            Self::Counter(c) => c.value,
            Self::Gauge(g) => g.value,
            Self::Histogram(h) => h.sum / h.count,
        }
    }

    pub fn key(&self) -> Vec<InternString> {
        let mut key = vec![self.name().clone()];
        for (k, v) in self.labels().iter() {
            key.push(k.clone());
            key.push(v.clone());
        }
        key
    }
}

#[derive(Debug, PartialEq)]
pub struct CounterMetric {
    pub name: InternString,
    pub labels: Map<InternString, InternString>,
    pub value: f64,
}

#[derive(Debug, PartialEq)]
pub struct GaugeMetric {
    pub name: InternString,
    pub labels: Map<InternString, InternString>,
    pub value: f64,
}

#[derive(Debug, PartialEq)]
pub struct HistogramMetric {
    pub name: InternString,
    pub labels: Map<InternString, InternString>,
    pub sum: f64,
    pub count: f64,
    pub bucket: Vec<(f64, f64)>,
}

#[cfg(test)]
mod test {
    use super::*;

    /*
    #[test]
    fn parse_it() {
        use std::collections::HashMap;
        use std::io::Read;
        let mut f = std::fs::File::open("/tmp/metrics").unwrap();
        let mut buf = [0u8; 8*1024];
        let mut parser = Parser::new();
        let mut map = HashMap::new();
        let mut num_metrics = 0;
        while let Ok(n) = f.read(&mut buf) {
            let is_final = n == 0;
            parser
                .push_bytes(&buf[0..n], is_final, |m| {
                    num_metrics += 1;
                    map.insert(m.key(), m);
                })
                .unwrap();
            if is_final {
                break;
            }
        }
        println!("There were {num_metrics} metrics");
    }
    */

    #[test]
    fn parse_counter() {
        let sample = r#"# HELP tokio_total_overflow_count The number of times worker threads saturated their local queues.
# TYPE tokio_total_overflow_count counter
tokio_total_overflow_count 0
"#;

        let mut parser = Parser::new();
        let metrics = parser.parse(sample).unwrap();
        assert_eq!(
            metrics,
            vec![Metric::Counter(CounterMetric {
                name: InternString::new("tokio_total_overflow_count"),
                labels: Map::new(),
                value: 0.0
            })]
        );
    }

    #[test]
    fn parse_gauge() {
        let sample = r#"# HELP lua_count the number of lua contexts currently alive
# TYPE lua_count gauge
lua_count 1
"#;

        let mut parser = Parser::new();
        let metrics = parser.parse(sample).unwrap();
        assert_eq!(
            metrics,
            vec![Metric::Gauge(GaugeMetric {
                name: InternString::new("lua_count"),
                labels: Map::new(),
                value: 1.0
            })]
        );
    }

    #[test]
    fn parse_histogram() {
        let sample = r#"# HELP deliver_message_latency_rollup how long a deliver_message call takes for a given protocol
# TYPE deliver_message_latency_rollup histogram
deliver_message_latency_rollup_bucket{service="smtp_client",le="0.005"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="0.01"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="0.025"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="0.05"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="0.1"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="0.25"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="0.5"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="1"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="2.5"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="5"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="10"} 0
deliver_message_latency_rollup_bucket{service="smtp_client",le="+Inf"} 0
deliver_message_latency_rollup_sum{service="smtp_client"} 0
deliver_message_latency_rollup_count{service="smtp_client"} 0
# HELP lua_event_latency how long a given lua event callback took
# TYPE lua_event_latency histogram
lua_event_latency_bucket{event="context-creation",le="0.005"} 5226
lua_event_latency_bucket{event="context-creation",le="0.01"} 5226
lua_event_latency_bucket{event="context-creation",le="0.025"} 5226
lua_event_latency_bucket{event="context-creation",le="0.05"} 5226
lua_event_latency_bucket{event="context-creation",le="0.1"} 5226
lua_event_latency_bucket{event="context-creation",le="0.25"} 5226
lua_event_latency_bucket{event="context-creation",le="0.5"} 5226
lua_event_latency_bucket{event="context-creation",le="1"} 5226
lua_event_latency_bucket{event="context-creation",le="2.5"} 5226
lua_event_latency_bucket{event="context-creation",le="5"} 5226
lua_event_latency_bucket{event="context-creation",le="10"} 5226
lua_event_latency_bucket{event="context-creation",le="+Inf"} 5226
lua_event_latency_sum{event="context-creation"} 7.057928427000033
lua_event_latency_count{event="context-creation"} 5226
lua_event_latency_bucket{event="get_egress_path_config",le="0.005"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="0.01"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="0.025"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="0.05"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="0.1"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="0.25"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="0.5"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="1"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="2.5"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="5"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="10"} 10
lua_event_latency_bucket{event="get_egress_path_config",le="+Inf"} 10
lua_event_latency_sum{event="get_egress_path_config"} 0.000493053
lua_event_latency_count{event="get_egress_path_config"} 10
"#;
        let mut parser = Parser::new();
        let metrics = parser.parse(sample).unwrap();
        assert_eq!(
            metrics,
            vec![
                Metric::Histogram(HistogramMetric {
                    name: InternString::new("deliver_message_latency_rollup"),
                    labels: [(
                        InternString::new("service"),
                        InternString::new("smtp_client")
                    )]
                    .into_iter()
                    .collect(),
                    sum: 0.0,
                    count: 0.0,
                    bucket: vec![
                        (0.005, 0.0),
                        (0.01, 0.0),
                        (0.025, 0.0),
                        (0.05, 0.0),
                        (0.1, 0.0),
                        (0.25, 0.0),
                        (0.5, 0.0),
                        (1.0, 0.0),
                        (2.5, 0.0),
                        (5.0, 0.0),
                        (10.0, 0.0),
                        (f64::INFINITY, 0.0)
                    ]
                }),
                Metric::Histogram(HistogramMetric {
                    name: InternString::new("lua_event_latency"),
                    labels: [(
                        InternString::new("event"),
                        InternString::new("context-creation")
                    )]
                    .into_iter()
                    .collect(),
                    sum: 7.057928427000033,
                    count: 5226.0,
                    bucket: vec![
                        (0.005, 5226.0),
                        (0.01, 5226.0),
                        (0.025, 5226.0),
                        (0.05, 5226.0),
                        (0.1, 5226.0),
                        (0.25, 5226.0),
                        (0.5, 5226.0),
                        (1.0, 5226.0),
                        (2.5, 5226.0),
                        (5.0, 5226.0),
                        (10.0, 5226.0),
                        (f64::INFINITY, 5226.0)
                    ],
                }),
                Metric::Histogram(HistogramMetric {
                    name: InternString::new("lua_event_latency"),
                    labels: [(
                        InternString::new("event"),
                        InternString::new("get_egress_path_config")
                    )]
                    .into_iter()
                    .collect(),
                    sum: 0.000493053,
                    count: 10.0,
                    bucket: vec![
                        (0.005, 10.0),
                        (0.01, 10.0),
                        (0.025, 10.0),
                        (0.05, 10.0),
                        (0.1, 10.0),
                        (0.25, 10.0),
                        (0.5, 10.0),
                        (1.0, 10.0),
                        (2.5, 10.0),
                        (5.0, 10.0),
                        (10.0, 10.0),
                        (f64::INFINITY, 10.0)
                    ],
                })
            ]
        );
    }

    #[test]
    fn parse_label_gauge() {
        let sample = r#"# HELP disk_free_bytes number of available bytes in a monitored location
# TYPE disk_free_bytes gauge
disk_free_bytes{name="data spool"} 1540683988992
disk_free_bytes{name="log dir /var/tmp/kumo-logs"} 1540683988992
disk_free_bytes{name="meta spool"} 1540683988992
"#;
        let mut parser = Parser::new();
        let metrics = parser.parse(sample).unwrap();
        assert_eq!(
            metrics,
            vec![
                Metric::Gauge(GaugeMetric {
                    name: InternString::new("disk_free_bytes"),
                    labels: [(InternString::new("name"), InternString::new("data spool"))]
                        .into_iter()
                        .collect(),
                    value: 1540683988992.0
                }),
                Metric::Gauge(GaugeMetric {
                    name: InternString::new("disk_free_bytes"),
                    labels: [(
                        InternString::new("name"),
                        InternString::new("log dir /var/tmp/kumo-logs")
                    )]
                    .into_iter()
                    .collect(),
                    value: 1540683988992.0
                }),
                Metric::Gauge(GaugeMetric {
                    name: InternString::new("disk_free_bytes"),
                    labels: [(InternString::new("name"), InternString::new("meta spool"))]
                        .into_iter()
                        .collect(),
                    value: 1540683988992.0
                }),
            ]
        );
    }
}
