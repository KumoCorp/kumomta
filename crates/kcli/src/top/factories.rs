use crate::top::accumulator::*;
use crate::top::{SeriesChartOptions, SeriesFactory, TimeSeries};
use kumo_prometheus::parser::Metric;

pub struct ThreadPoolFactory {}

impl SeriesFactory for ThreadPoolFactory {
    fn matches(&self, metric: &Metric) -> Option<String> {
        match metric.name().as_str() {
            "thread_pool_size" | "thread_pool_parked" => {
                metric.labels().get("pool").map(|s| format!("pool {s}"))
            }
            _ => None,
        }
    }

    fn factory(&self, series_name: &str, metric: &Metric) -> TimeSeries {
        let pool = metric.labels().get("pool").unwrap();
        let mut series = TimeSeries::new(ThreadPoolAccumulator::new(&pool));

        series.set_chart(SeriesChartOptions {
            name: series_name.to_string(),
            inverted: false,
            unit: "%".to_string(),
        });

        series
    }
}

pub struct HistogramEventFreqFactory {}

impl SeriesFactory for HistogramEventFreqFactory {
    fn matches(&self, metric: &Metric) -> Option<String> {
        if metric.is_histogram() {
            let h = metric.as_histogram();

            let label = match h.labels.values().next() {
                Some(l) => l.as_str(),
                None => h.name.as_str(),
            };

            if label == "init" || label == "pre_init" {
                return None;
            }

            Some(format!("freq: {label}"))
        } else {
            None
        }
    }

    fn factory(&self, series_name: &str, metric: &Metric) -> TimeSeries {
        let h = metric.as_histogram();
        let mut count_series = match h.labels.iter().next() {
            Some((key, value)) => DirectAccumulator::new_with_label_match(
                metric.name().to_string(),
                key.to_string(),
                value.to_string(),
            ),
            None => DirectAccumulator::new(metric.name().to_string()),
        };
        count_series.set_target(AccumulatorTarget::HistogramCount);

        let mut series = TimeSeries::new(RateAccumulator::new(count_series));

        series.set_chart(SeriesChartOptions {
            name: series_name.to_string(),
            inverted: false,
            unit: "/s".to_string(),
        });

        series
    }
}

pub struct HistogramEventAvgFactory {}

impl SeriesFactory for HistogramEventAvgFactory {
    fn matches(&self, metric: &Metric) -> Option<String> {
        if metric.is_histogram() {
            let h = metric.as_histogram();

            let label = match h.labels.values().next() {
                Some(l) => l.as_str(),
                None => h.name.as_str(),
            };

            if label == "init" || label == "pre_init" {
                return None;
            }

            Some(format!("avg: {label}"))
        } else {
            None
        }
    }

    fn factory(&self, series_name: &str, metric: &Metric) -> TimeSeries {
        let h = metric.as_histogram();
        let mut count_series = match h.labels.iter().next() {
            Some((key, value)) => DirectAccumulator::new_with_label_match(
                metric.name().to_string(),
                key.to_string(),
                value.to_string(),
            ),
            None => DirectAccumulator::new(metric.name().to_string()),
        };
        // The "Value" of a histogram is sum / count which == avg over its lifetime
        count_series.set_target(AccumulatorTarget::Value);
        count_series.set_scale(1_000_000.0);

        let mut series = TimeSeries::new(count_series);

        series.set_chart(SeriesChartOptions {
            name: series_name.to_string(),
            inverted: false,
            unit: "us".to_string(),
        });

        series
    }
}
