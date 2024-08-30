use kumo_prometheus::{label_key, AtomicCounter, CounterRegistry, PruningCounterRegistry};
use once_cell::sync::Lazy;
use prometheus::{Histogram, HistogramVec, IntCounter};

label_key! {
    pub struct ServiceKey {
        pub service: String,
    }
}

pub static CONN_GAUGE: Lazy<PruningCounterRegistry<ServiceKey>> = Lazy::new(|| {
    PruningCounterRegistry::register_gauge("connection_count", "number of active connections")
});
pub static CONN_DENIED: Lazy<PruningCounterRegistry<ServiceKey>> = Lazy::new(|| {
    PruningCounterRegistry::register(
        "total_connections_denied",
        "total number of connections rejected due to load shedding or concurrency limits",
    )
});

pub static TOTAL_CONN: Lazy<PruningCounterRegistry<ServiceKey>> = Lazy::new(|| {
    PruningCounterRegistry::register(
        "total_connection_count",
        "total number of active connections ever made",
    )
});

pub static TOTAL_MSGS_DELIVERED: Lazy<PruningCounterRegistry<ServiceKey>> = Lazy::new(|| {
    PruningCounterRegistry::register(
        "total_messages_delivered",
        "total number of messages ever delivered",
    )
});
pub static TOTAL_MSGS_TRANSFAIL: Lazy<PruningCounterRegistry<ServiceKey>> = Lazy::new(|| {
    PruningCounterRegistry::register(
        "total_messages_transfail",
        "total number of message delivery attempts that transiently failed",
    )
});
pub static TOTAL_MSGS_FAIL: Lazy<PruningCounterRegistry<ServiceKey>> = Lazy::new(|| {
    PruningCounterRegistry::register(
        "total_messages_fail",
        "total number of message delivery attempts that permanently failed",
    )
});
pub static READY_COUNT_GAUGE: Lazy<PruningCounterRegistry<ServiceKey>> = Lazy::new(|| {
    PruningCounterRegistry::register("ready_count", "number of messages in the ready queue")
});
pub static TOTAL_MSGS_RECVD: Lazy<CounterRegistry<ServiceKey>> = Lazy::new(|| {
    CounterRegistry::register(
        "total_messages_received",
        "total number of messages ever received",
    )
});
pub static READY_FULL_COUNTER: Lazy<PruningCounterRegistry<ServiceKey>> = Lazy::new(|| {
    PruningCounterRegistry::register(
        "ready_full",
        "number of times a message could not fit in the ready queue",
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

pub fn connection_denied_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    CONN_DENIED.get_or_create(&service as &dyn ServiceKeyTrait)
}

pub fn ready_full_counter_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    READY_FULL_COUNTER.get_or_create(&service as &dyn ServiceKeyTrait)
}

pub fn ready_count_gauge_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    READY_COUNT_GAUGE.get_or_create(&service as &dyn ServiceKeyTrait)
}

pub fn connection_gauge_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    CONN_GAUGE.get_or_create(&service as &dyn ServiceKeyTrait)
}

pub fn connection_total_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    TOTAL_CONN.get_or_create(&service as &dyn ServiceKeyTrait)
}

pub fn total_msgs_received_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    TOTAL_MSGS_RECVD.get_or_create(&service as &dyn ServiceKeyTrait)
}

pub fn total_msgs_delivered_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    TOTAL_MSGS_DELIVERED.get_or_create(&service as &dyn ServiceKeyTrait)
}

pub fn total_msgs_transfail_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    TOTAL_MSGS_TRANSFAIL.get_or_create(&service as &dyn ServiceKeyTrait)
}

pub fn total_msgs_fail_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    TOTAL_MSGS_FAIL.get_or_create(&service as &dyn ServiceKeyTrait)
}
