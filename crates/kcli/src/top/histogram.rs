use kumo_prometheus::parser::Metric;

#[derive(Debug, PartialEq)]
pub struct Histogram {
    pub data: Vec<Vec<u64>>,
    pub buckets: Vec<f64>,
    name: String,
    label: Option<String>,
    label_value: Option<String>,
    pub unit: String,
}

impl Histogram {
    pub fn new<N: Into<String>>(name: N, unit: &str) -> Self {
        Self {
            data: vec![],
            buckets: vec![],
            name: name.into(),
            label: None,
            label_value: None,
            unit: unit.to_string(),
        }
    }

    pub fn new_with_label_match<N: Into<String>, L: Into<String>, V: Into<String>>(
        name: N,
        label: L,
        label_value: V,
        unit: &str,
    ) -> Self {
        Self {
            data: vec![],
            buckets: vec![],
            name: name.into(),
            label: Some(label.into()),
            label_value: Some(label_value.into()),
            unit: unit.to_string(),
        }
    }

    pub fn accumulate(&mut self, metric: &Metric) {
        if metric.name().as_str() != self.name.as_str() {
            return;
        }
        if let (Some(label_name), Some(label_value)) = (&self.label, &self.label_value) {
            if !metric.label_is(label_name, label_value) {
                return;
            }
        }
        match metric {
            Metric::Histogram(histo) => {
                // Note that we assume that histo.bucket is ordered by threshold.
                // This is currently guaranteed by the kumo prometheus metric
                // exporter, so we don't need to fix it up on the client side.
                let mut data = vec![];
                let need_buckets = self.buckets.is_empty();
                let mut buckets = vec![];
                for (thresh, value) in &histo.bucket {
                    if need_buckets {
                        buckets.push(*thresh);
                    }
                    data.push(*value as u64);
                }

                if need_buckets {
                    self.buckets = buckets;
                }

                self.data.push(data);
            }
            _ => unreachable!(),
        }
    }
}
