use prometheus::{IntGauge, IntGaugeVec};

lazy_static::lazy_static! {
    pub static ref CONN_GAUGE: IntGaugeVec = {
        prometheus::register_int_gauge_vec!(
            "connection_count",
            "number of active connections",
            &["service"]).unwrap()
    };
}

pub fn connection_gauge_for_service(service: &str) -> IntGauge {
    CONN_GAUGE.get_metric_with_label_values(&[service]).unwrap()
}

/// Remove metrics that are parameterized by a service name of
/// some kind.
///
/// The rationale is that, at scale, we may instantiate many
/// thousands, or even millions, of objects for different
/// destination sites on the internet: the worst case metric
/// is O(number-of-internet-domains).
/// If we only ever deliver a single message to each domain
/// we'd be paying a huge RAM cost for tracking the metrics
/// from when we did that.
///
/// When we idle out after a sufficient period, we want to
/// prune such structures and their associated metrics.
/// This function should be called at that time.
pub fn remove_metrics_for_service(service: &str) {
    CONN_GAUGE.remove_label_values(&[service]).ok();
}
