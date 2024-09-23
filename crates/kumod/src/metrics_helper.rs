use kumo_prometheus::{label_key, AtomicCounter, CounterRegistry, PruningCounterRegistry};
use prometheus::{Histogram, HistogramVec, IntCounter};
use std::sync::LazyLock;

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

pub static CONN_GAUGE: LazyLock<PruningCounterRegistry<ServiceKey>> = LazyLock::new(|| {
    PruningCounterRegistry::register_gauge("connection_count", "number of active connections")
});
pub static CONN_GAUGE_BY_PROVIDER: LazyLock<PruningCounterRegistry<ProviderKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register_gauge(
            "connection_count_by_provider",
            "number of active connections",
        )
    });
pub static CONN_GAUGE_BY_PROVIDER_AND_POOL: LazyLock<PruningCounterRegistry<ProviderAndPoolKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register_gauge(
            "connection_count_by_provider_and_pool",
            "number of active connections",
        )
    });
pub static CONN_DENIED: LazyLock<PruningCounterRegistry<ServiceKey>> = LazyLock::new(|| {
    PruningCounterRegistry::register(
        "total_connections_denied",
        "total number of connections rejected due to load shedding or concurrency limits",
    )
});

pub static TOTAL_CONN: LazyLock<PruningCounterRegistry<ServiceKey>> = LazyLock::new(|| {
    PruningCounterRegistry::register(
        "total_connection_count",
        "total number of active connections ever made",
    )
});

pub static TOTAL_MSGS_DELIVERED: LazyLock<PruningCounterRegistry<ServiceKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register(
            "total_messages_delivered",
            "total number of messages ever delivered",
        )
    });
pub static TOTAL_MSGS_TRANSFAIL: LazyLock<PruningCounterRegistry<ServiceKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register(
            "total_messages_transfail",
            "total number of message delivery attempts that transiently failed",
        )
    });
pub static TOTAL_MSGS_FAIL: LazyLock<PruningCounterRegistry<ServiceKey>> = LazyLock::new(|| {
    PruningCounterRegistry::register(
        "total_messages_fail",
        "total number of message delivery attempts that permanently failed",
    )
});

pub static TOTAL_MSGS_DELIVERED_BY_PROVIDER: LazyLock<PruningCounterRegistry<ProviderKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register(
            "total_messages_delivered_by_provider",
            "total number of messages ever delivered",
        )
    });
pub static TOTAL_MSGS_TRANSFAIL_BY_PROVIDER: LazyLock<PruningCounterRegistry<ProviderKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register(
            "total_messages_transfail_by_provider",
            "total number of message delivery attempts that transiently failed",
        )
    });
pub static TOTAL_MSGS_FAIL_BY_PROVIDER: LazyLock<PruningCounterRegistry<ProviderKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register(
            "total_messages_fail_by_provider",
            "total number of message delivery attempts that permanently failed",
        )
    });

pub static TOTAL_MSGS_DELIVERED_BY_PROVIDER_AND_SOURCE: LazyLock<
    PruningCounterRegistry<ProviderAndSourceKey>,
> = LazyLock::new(|| {
    PruningCounterRegistry::register(
        "total_messages_delivered_by_provider_and_source",
        "total number of messages ever delivered",
    )
});
pub static TOTAL_MSGS_TRANSFAIL_BY_PROVIDER_AND_SOURCE: LazyLock<
    PruningCounterRegistry<ProviderAndSourceKey>,
> = LazyLock::new(|| {
    PruningCounterRegistry::register(
        "total_messages_transfail_by_provider_and_source",
        "total number of message delivery attempts that transiently failed",
    )
});
pub static TOTAL_MSGS_FAIL_BY_PROVIDER_AND_SOURCE: LazyLock<
    PruningCounterRegistry<ProviderAndSourceKey>,
> = LazyLock::new(|| {
    PruningCounterRegistry::register(
        "total_messages_fail_by_provider_and_source",
        "total number of message delivery attempts that permanently failed",
    )
});

pub static READY_COUNT_GAUGE: LazyLock<PruningCounterRegistry<ServiceKey>> = LazyLock::new(|| {
    PruningCounterRegistry::register_gauge("ready_count", "number of messages in the ready queue")
});

pub static QUEUED_COUNT_GAUGE_BY_PROVIDER: LazyLock<PruningCounterRegistry<ProviderKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register_gauge(
            "queued_count_by_provider",
            "number of messages in the scheduled and ready queue",
        )
    });
pub static QUEUED_COUNT_GAUGE_BY_PROVIDER_AND_POOL: LazyLock<
    PruningCounterRegistry<ProviderAndPoolKey>,
> = LazyLock::new(|| {
    PruningCounterRegistry::register_gauge(
        "queued_count_by_provider_and_pool",
        "number of messages in the scheduled and ready queue",
    )
});

pub static TOTAL_MSGS_RECVD: LazyLock<CounterRegistry<ServiceKey>> = LazyLock::new(|| {
    CounterRegistry::register(
        "total_messages_received",
        "total number of messages ever received",
    )
});
pub static READY_FULL_COUNTER: LazyLock<PruningCounterRegistry<ServiceKey>> = LazyLock::new(|| {
    PruningCounterRegistry::register(
        "ready_full",
        "number of times a message could not fit in the ready queue",
    )
});
pub static DELIVER_MESSAGE_LATENCY_ROLLUP: LazyLock<HistogramVec> = LazyLock::new(|| {
    prometheus::register_histogram_vec!(
        "deliver_message_latency_rollup",
        "how long a deliver_message call takes for a given protocol",
        &["service"]
    )
    .unwrap()
});
pub static TOTAL_READYQ_RUNS: LazyLock<IntCounter> = LazyLock::new(|| {
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
