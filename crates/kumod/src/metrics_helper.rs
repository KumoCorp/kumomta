use kumo_prometheus::{
    PruningIntCounter, PruningIntCounterVec, PruningIntGauge, PruningIntGaugeVec,
};
use once_cell::sync::Lazy;
use prometheus::{Histogram, HistogramVec, IntCounter};

pub static CONN_GAUGE: Lazy<PruningIntGaugeVec> = Lazy::new(|| {
    PruningIntGaugeVec::register(
        "connection_count",
        "number of active connections",
        &["service"],
    )
});
pub static CONN_DENIED: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "total_connections_denied",
        "total number of connections rejected due to load shedding or concurrency limits",
        &["service"],
    )
});

pub static TOTAL_CONN: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "total_connection_count",
        "total number of active connections ever made",
        &["service"],
    )
});

pub static TOTAL_MSGS_DELIVERED: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "total_messages_delivered",
        "total number of messages ever delivered",
        &["service"],
    )
});
pub static TOTAL_MSGS_TRANSFAIL: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "total_messages_transfail",
        "total number of message delivery attempts that transiently failed",
        &["service"],
    )
});
pub static TOTAL_MSGS_FAIL: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "total_messages_fail",
        "total number of message delivery attempts that permanently failed",
        &["service"],
    )
});
pub static READY_COUNT_GAUGE: Lazy<PruningIntGaugeVec> = Lazy::new(|| {
    PruningIntGaugeVec::register(
        "ready_count",
        "number of messages in the ready queue",
        &["service"],
    )
});
pub static TOTAL_MSGS_RECVD: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "total_messages_received",
        "total number of messages ever received",
        &["service"],
    )
});
pub static READY_FULL_COUNTER: Lazy<PruningIntCounterVec> = Lazy::new(|| {
    PruningIntCounterVec::register(
        "ready_full",
        "number of times a message could not fit in the ready queue",
        &["service"],
    )
});
pub static DELIVER_MESSAGE_LATENCY_ROLLUP: Lazy<HistogramVec> = Lazy::new(|| {
    prometheus::register_histogram_vec!(
        "deliver_message_latency_rollup",
        "how long a deliver_message call takes for a given protocol",
        &["service"]
    )
    .unwrap()
});
pub static TOTAL_READYQ_RUNS: Lazy<IntCounter> = Lazy::new(|| {
    prometheus::register_int_counter!(
        "total_readyq_runs",
        "total number of times a readyq maintainer was run"
    )
    .unwrap()
});

pub fn deliver_message_rollup_for_service(service: &str) -> Histogram {
    DELIVER_MESSAGE_LATENCY_ROLLUP
        .get_metric_with_label_values(&[service])
        .unwrap()
}

pub fn connection_denied_for_service(service: &str) -> PruningIntCounter {
    CONN_DENIED.with_label_values(&[service])
}

pub fn ready_full_counter_for_service(service: &str) -> PruningIntCounter {
    READY_FULL_COUNTER.with_label_values(&[service])
}

pub fn ready_count_gauge_for_service(service: &str) -> PruningIntGauge {
    READY_COUNT_GAUGE.with_label_values(&[service])
}

pub fn connection_gauge_for_service(service: &str) -> PruningIntGauge {
    CONN_GAUGE.with_label_values(&[service])
}

pub fn connection_total_for_service(service: &str) -> PruningIntCounter {
    TOTAL_CONN.with_label_values(&[service])
}

pub fn total_msgs_received_for_service(service: &str) -> PruningIntCounter {
    TOTAL_MSGS_RECVD.with_label_values(&[service])
}

pub fn total_msgs_delivered_for_service(service: &str) -> PruningIntCounter {
    TOTAL_MSGS_DELIVERED.with_label_values(&[service])
}

pub fn total_msgs_transfail_for_service(service: &str) -> PruningIntCounter {
    TOTAL_MSGS_TRANSFAIL.with_label_values(&[service])
}

pub fn total_msgs_fail_for_service(service: &str) -> PruningIntCounter {
    TOTAL_MSGS_FAIL.with_label_values(&[service])
}
