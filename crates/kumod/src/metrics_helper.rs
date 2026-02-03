use kumo_prometheus::prometheus::Histogram;
use kumo_prometheus::{declare_metric, label_key, AtomicCounter};

label_key! {
    pub struct ServiceKey {
        pub service: String,
    }
}
label_key! {
    pub struct ProviderKey {
        pub provider: String,
    }
}
label_key! {
    pub struct ProviderAndSourceKey {
        pub provider: String,
        pub source: String,
        pub pool: String,
    }
}
label_key! {
    pub struct ProviderAndPoolKey {
        pub provider: String,
        pub pool: String,
    }
}

declare_metric! {
/// The number of active outgoing connections in the system,
/// keyed by the service name.
pub static CONN_GAUGE: PruningGaugeRegistry<ServiceKey>("connection_count");
}

declare_metric! {
/// number of active connections
pub static CONN_GAUGE_BY_PROVIDER: PruningGaugeRegistry<ProviderKey>(
    "connection_count_by_provider");
}

declare_metric! {
/// number of active connections
pub static CONN_GAUGE_BY_PROVIDER_AND_POOL:
    PruningGaugeRegistry<ProviderAndPoolKey>("connection_count_by_provider_and_pool");
}

declare_metric! {
/// number of Rejection records logged by the smtp server
pub static SMTP_SERVER_REJECTIONS: CounterRegistry<ServiceKey>("smtp_server_rejections");
}

declare_metric! {
/// total number of connections rejected due to load shedding or concurrency limits
pub static CONN_DENIED: PruningCounterRegistry<ServiceKey>("total_connections_denied");
}

declare_metric! {
/// total number of active connections ever made
pub static TOTAL_CONN: PruningCounterRegistry<ServiceKey>("total_connection_count");
}

declare_metric! {
/// total number of messages ever delivered
pub static TOTAL_MSGS_DELIVERED: PruningCounterRegistry<ServiceKey>("total_messages_delivered");
}

declare_metric! {
/// total number of message delivery attempts that transiently failed
pub static TOTAL_MSGS_TRANSFAIL: PruningCounterRegistry<ServiceKey>("total_messages_transfail");
}

declare_metric! {
/// total number of message delivery attempts that permanently failed
pub static TOTAL_MSGS_FAIL: PruningCounterRegistry<ServiceKey>("total_messages_fail");
}

declare_metric! {
/// total number of messages ever delivered
pub static TOTAL_MSGS_DELIVERED_BY_PROVIDER: PruningCounterRegistry<ProviderKey>(
        "total_messages_delivered_by_provider");
}

declare_metric! {
/// total number of message delivery attempts that transiently failed
pub static TOTAL_MSGS_TRANSFAIL_BY_PROVIDER: PruningCounterRegistry<ProviderKey>(
        "total_messages_transfail_by_provider");
}

declare_metric! {
/// total number of message delivery attempts that permanently failed
pub static TOTAL_MSGS_FAIL_BY_PROVIDER: PruningCounterRegistry<ProviderKey>(
        "total_messages_fail_by_provider");
}

declare_metric! {
/// total number of messages ever delivered
pub static TOTAL_MSGS_DELIVERED_BY_PROVIDER_AND_SOURCE:
    PruningCounterRegistry<ProviderAndSourceKey>(
        "total_messages_delivered_by_provider_and_source");
}

declare_metric! {
/// total number of message delivery attempts that transiently failed
pub static TOTAL_MSGS_TRANSFAIL_BY_PROVIDER_AND_SOURCE:
    PruningCounterRegistry<ProviderAndSourceKey>(
        "total_messages_transfail_by_provider_and_source");
}

declare_metric! {
/// total number of message delivery attempts that permanently failed
pub static TOTAL_MSGS_FAIL_BY_PROVIDER_AND_SOURCE:
    PruningCounterRegistry<ProviderAndSourceKey>(
        "total_messages_fail_by_provider_and_source");
}

declare_metric! {
/// number of messages in the ready queue
pub static READY_COUNT_GAUGE: PruningGaugeRegistry<ServiceKey>(
    "ready_count");
}

declare_metric! {
/// number of messages in the scheduled and ready queue
pub static QUEUED_COUNT_GAUGE_BY_PROVIDER: PruningGaugeRegistry<ProviderKey>(
            "queued_count_by_provider");
}

declare_metric! {
/// number of messages in the scheduled and ready queue
pub static QUEUED_COUNT_GAUGE_BY_PROVIDER_AND_POOL:
    PruningGaugeRegistry<ProviderAndPoolKey>("queued_count_by_provider_and_pool");
}

declare_metric! {
/// total number of messages ever received
pub static TOTAL_MSGS_RECVD: CounterRegistry<ServiceKey>("total_messages_received");
}

declare_metric! {
/// number of times a message could not fit in the ready queue
pub static READY_FULL_COUNTER: PruningCounterRegistry<ServiceKey>("ready_full");
}

declare_metric! {
/// how long a deliver_message call takes for a given protocol
pub static DELIVER_MESSAGE_LATENCY_ROLLUP: HistogramVec(
        "deliver_message_latency_rollup",
        &["service"]
    );
}

declare_metric! {
/// total number of times a readyq maintainer was run
pub static TOTAL_READYQ_RUNS: IntCounter("total_readyq_runs");
}

pub fn deliver_message_rollup_for_service(service: &str) -> Histogram {
    DELIVER_MESSAGE_LATENCY_ROLLUP
        .get_metric_with_label_values(&[service])
        .unwrap()
}

pub fn smtp_rejected_for_service(service: &str) -> AtomicCounter {
    let service = BorrowedServiceKey { service };
    SMTP_SERVER_REJECTIONS.get_or_create(&service as &dyn ServiceKeyTrait)
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
