use prometheus::{IntCounter, IntCounterVec, IntGauge, IntGaugeVec};

lazy_static::lazy_static! {
    pub static ref CONN_GAUGE: IntGaugeVec = {
        prometheus::register_int_gauge_vec!(
            "connection_count",
            "number of active connections",
            &["service"]).unwrap()
    };
    pub static ref TOTAL_CONN: IntCounterVec = {
        prometheus::register_int_counter_vec!(
            "total_connection_count",
            "total number of active connections ever made",
            &["service"]).unwrap()
    };
    pub static ref TOTAL_MSGS_DELIVERED: IntCounterVec = {
        prometheus::register_int_counter_vec!(
            "total_messages_delivered",
            "total number of messages ever delivered",
            &["service"]).unwrap()
    };
    pub static ref TOTAL_MSGS_TRANSFAIL: IntCounterVec = {
        prometheus::register_int_counter_vec!(
            "total_messages_transfail",
            "total number of message delivery attempts that transiently failed",
            &["service"]).unwrap()
    };
    pub static ref TOTAL_MSGS_FAIL: IntCounterVec = {
        prometheus::register_int_counter_vec!(
            "total_messages_fail",
            "total number of message delivery attempts that permanently failed",
            &["service"]).unwrap()
    };
    pub static ref READY_COUNT_GAUGE: IntGaugeVec = {
        prometheus::register_int_gauge_vec!(
            "ready_count",
            "number of messages in the ready queue",
            &["service"]).unwrap()
    };
    pub static ref TOTAL_MSGS_RECVD: IntCounterVec = {
        prometheus::register_int_counter_vec!(
            "total_messages_received",
            "total number of messages ever received",
            &["service"]).unwrap()
    };
    pub static ref READY_FULL_COUNTER: IntCounterVec = {
        prometheus::register_int_counter_vec!(
            "ready_full",
            "number of times a message could not fit in the ready queue",
            &["service"]).unwrap()
    };
}

pub fn ready_full_counter_for_service(service: &str) -> IntCounter {
    READY_FULL_COUNTER
        .get_metric_with_label_values(&[service])
        .unwrap()
}

pub fn ready_count_gauge_for_service(service: &str) -> IntGauge {
    READY_COUNT_GAUGE
        .get_metric_with_label_values(&[service])
        .unwrap()
}

pub fn connection_gauge_for_service(service: &str) -> IntGauge {
    CONN_GAUGE.get_metric_with_label_values(&[service]).unwrap()
}

pub fn connection_total_for_service(service: &str) -> IntCounter {
    TOTAL_CONN.get_metric_with_label_values(&[service]).unwrap()
}

pub fn total_msgs_received_for_service(service: &str) -> IntCounter {
    TOTAL_MSGS_RECVD
        .get_metric_with_label_values(&[service])
        .unwrap()
}

pub fn total_msgs_delivered_for_service(service: &str) -> IntCounter {
    TOTAL_MSGS_DELIVERED
        .get_metric_with_label_values(&[service])
        .unwrap()
}

pub fn total_msgs_transfail_for_service(service: &str) -> IntCounter {
    TOTAL_MSGS_TRANSFAIL
        .get_metric_with_label_values(&[service])
        .unwrap()
}

pub fn total_msgs_fail_for_service(service: &str) -> IntCounter {
    TOTAL_MSGS_FAIL
        .get_metric_with_label_values(&[service])
        .unwrap()
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
    TOTAL_CONN.remove_label_values(&[service]).ok();
    TOTAL_MSGS_DELIVERED.remove_label_values(&[service]).ok();
    TOTAL_MSGS_TRANSFAIL.remove_label_values(&[service]).ok();
    TOTAL_MSGS_FAIL.remove_label_values(&[service]).ok();
    READY_COUNT_GAUGE.remove_label_values(&[service]).ok();
    READY_FULL_COUNTER.remove_label_values(&[service]).ok();
}
