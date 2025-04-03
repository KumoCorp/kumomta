use crate::top::accumulator::*;
use kumo_prometheus::parser::Metric;

pub struct SeriesChartOptions {
    pub name: String,
    pub inverted: bool,
    pub unit: String,
}

pub struct TimeSeries {
    pub data: Vec<u64>,
    accumulator: Box<dyn Accumulator + 'static>,
    pub chart: Option<SeriesChartOptions>,
}

impl TimeSeries {
    pub fn new<A: Accumulator + 'static>(accumulator: A) -> Self {
        Self {
            data: vec![],
            accumulator: Box::new(accumulator),
            chart: None,
        }
    }

    pub fn set_chart(&mut self, chart: SeriesChartOptions) {
        self.chart.replace(chart);
    }

    pub fn accumulate(&mut self, metric: &Metric) {
        self.accumulator.accumulate(metric);
    }

    pub fn commit(&mut self) {
        let value = self.accumulator.commit();
        self.data.insert(0, value as u64);
        self.data.truncate(1024);
    }
}

pub trait SeriesFactory {
    /// Returns the series name that should be created for this metric
    fn matches(&self, metric: &Metric) -> Option<String>;
    /// Constructs the appropriate series for this metric
    fn factory(&self, series_name: &str, metric: &Metric) -> TimeSeries;
}
