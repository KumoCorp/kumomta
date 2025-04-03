use kumo_prometheus::parser::Metric;
use tokio::time::Instant;

#[allow(unused)]
pub enum AccumulatorTarget {
    Value,
    HistogramCount,
    HistogramSum,
    HistogramQuantile(f64),
}

pub trait Accumulator {
    fn accumulate(&mut self, metric: &Metric);
    fn commit(&mut self) -> f64;
}

impl AccumulatorTarget {
    pub fn get_value(&self, metric: &Metric) -> f64 {
        match self {
            AccumulatorTarget::Value => metric.value(),
            AccumulatorTarget::HistogramCount => metric.as_histogram().count,
            AccumulatorTarget::HistogramSum => metric.as_histogram().sum,
            AccumulatorTarget::HistogramQuantile(n) => metric.as_histogram().quantile(*n),
        }
    }
}

pub struct DirectAccumulator {
    pub name: String,
    pub label: Option<String>,
    pub label_value: Option<String>,
    pub value: Option<f64>,
    pub target: AccumulatorTarget,
    pub scale: f64,
}

impl DirectAccumulator {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            label: None,
            label_value: None,
            value: None,
            target: AccumulatorTarget::Value,
            scale: 1.0,
        }
    }

    pub fn new_with_label_match<N: Into<String>, L: Into<String>, V: Into<String>>(
        name: N,
        label: L,
        label_value: V,
    ) -> Self {
        Self {
            name: name.into(),
            label: Some(label.into()),
            label_value: Some(label_value.into()),
            value: None,
            target: AccumulatorTarget::Value,
            scale: 1.0,
        }
    }

    pub fn set_target(&mut self, target: AccumulatorTarget) {
        self.target = target;
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
    }
}

impl Accumulator for DirectAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        if metric.name().as_str() == self.name.as_str() {
            match (&self.label, &self.label_value) {
                (Some(label_name), Some(label_value)) => {
                    if metric.label_is(label_name, label_value) {
                        self.value
                            .replace(self.target.get_value(metric) * self.scale);
                    }
                }
                _ => {
                    self.value
                        .replace(self.target.get_value(metric) * self.scale);
                }
            }
        }
    }

    fn commit(&mut self) -> f64 {
        self.value.take().unwrap_or(0.0)
    }
}

pub struct SumMultipleAccumulator {
    accumulators: Vec<Box<dyn Accumulator + 'static>>,
}

impl SumMultipleAccumulator {
    pub fn new(accumulators: Vec<Box<dyn Accumulator + 'static>>) -> Self {
        Self { accumulators }
    }
}

impl Accumulator for SumMultipleAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        for a in &mut self.accumulators {
            a.accumulate(metric);
        }
    }
    fn commit(&mut self) -> f64 {
        let mut result = 0.0;
        for a in &mut self.accumulators {
            result += a.commit();
        }
        result
    }
}

pub struct SummingAccumulator {
    pub name: String,
    pub value: Option<f64>,
    pub filter: Box<dyn Fn(&Metric) -> bool>,
}

impl SummingAccumulator {
    pub fn new<S: Into<String>, F: Fn(&Metric) -> bool + 'static>(name: S, filter: F) -> Self {
        Self {
            name: name.into(),
            value: None,
            filter: Box::new(filter),
        }
    }
}

impl Accumulator for SummingAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        if metric.name().as_str() == self.name.as_str() {
            if !(self.filter)(metric) {
                return;
            }
            let value = self.value.take().unwrap_or(0.) + metric.value();
            self.value.replace(value);
        }
    }

    fn commit(&mut self) -> f64 {
        self.value.take().unwrap_or(0.)
    }
}

pub struct RateAccumulator {
    accumulator: Box<dyn Accumulator + 'static>,
    prior: Option<(Instant, f64)>,
}

impl RateAccumulator {
    pub fn new<A: Accumulator + 'static>(accumulator: A) -> Self {
        Self {
            accumulator: Box::new(accumulator),
            prior: None,
        }
    }
}

impl Accumulator for RateAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        self.accumulator.accumulate(metric);
    }

    fn commit(&mut self) -> f64 {
        let value = self.accumulator.commit();

        let result = if let Some((when, prior_value)) = self.prior.take() {
            let elapsed = when.elapsed().as_secs_f64();

            (value - prior_value) / elapsed
        } else {
            0.0
        };

        self.prior.replace((Instant::now(), value));

        result
    }
}

pub struct ThreadPoolAccumulator {
    pool: String,
    size: f64,
    parked: f64,
}

impl ThreadPoolAccumulator {
    pub fn new(pool: &str) -> Self {
        Self {
            pool: pool.to_string(),
            size: 0.,
            parked: 0.,
        }
    }
}

impl Accumulator for ThreadPoolAccumulator {
    fn accumulate(&mut self, metric: &Metric) {
        match metric.name().as_str() {
            "thread_pool_size" => {
                if metric.label_is("pool", &self.pool) {
                    self.size = metric.value();
                }
            }
            "thread_pool_parked" => {
                if metric.label_is("pool", &self.pool) {
                    self.parked = metric.value();
                }
            }
            _ => {}
        }
    }

    fn commit(&mut self) -> f64 {
        if self.size == 0.0 {
            0.0
        } else {
            let utilization_percent = (100. * (self.size - self.parked)) / self.size;
            self.parked = self.size;
            self.size = 0.0;
            utilization_percent
        }
    }
}
