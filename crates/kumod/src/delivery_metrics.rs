use crate::metrics_helper::{
    BorrowedProviderAndPoolKey, BorrowedProviderAndSourceKey, BorrowedProviderKey,
    ProviderAndPoolKeyTrait, ProviderAndSourceKeyTrait, ProviderKeyTrait,
    QUEUED_COUNT_GAUGE_BY_PROVIDER, QUEUED_COUNT_GAUGE_BY_PROVIDER_AND_POOL,
    TOTAL_MSGS_DELIVERED_BY_PROVIDER, TOTAL_MSGS_DELIVERED_BY_PROVIDER_AND_SOURCE,
    TOTAL_MSGS_FAIL_BY_PROVIDER, TOTAL_MSGS_FAIL_BY_PROVIDER_AND_SOURCE,
    TOTAL_MSGS_TRANSFAIL_BY_PROVIDER, TOTAL_MSGS_TRANSFAIL_BY_PROVIDER_AND_SOURCE,
};
use kumo_prometheus::{counter_bundle, AtomicCounter};
use parking_lot::Mutex;
use prometheus::Histogram;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

counter_bundle! {
    pub struct ReadyCountBundle {
        pub ready_count_by_service: AtomicCounter,
        pub global_ready_count: AtomicCounter,
        pub queued_by_provider: AtomicCounter,
        pub queued_by_provider_and_pool: AtomicCounter,
    }
}
counter_bundle! {
    pub struct DispositionBundle {
        pub msgs: AtomicCounter,
        pub provider: AtomicCounter,
        pub source_provider: AtomicCounter,
        pub global: AtomicCounter,
    }
}
counter_bundle! {
    pub struct ConnectionGaugeBundle {
        pub connections: AtomicCounter,
        pub global: AtomicCounter,
        pub provider: AtomicCounter,
        pub provider_and_pool: AtomicCounter,
    }
}

#[derive(Clone)]
pub struct DeliveryMetrics {
    connection_gauge: ConnectionGaugeBundle,
    connection_total: AtomicCounter,
    global_connection_total: AtomicCounter,

    pub ready_count: ReadyCountBundle,
    pub ready_full: AtomicCounter,

    delivered: DispositionBundle,
    transfail: DispositionBundle,
    fail: DispositionBundle,

    pub deliver_message_rollup: Histogram,
}

impl std::fmt::Debug for DeliveryMetrics {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("DeliveryMetrics").finish()
    }
}

impl DeliveryMetrics {
    pub fn wrap_connection<T>(&self, client: T) -> MetricsWrappedConnection<T> {
        self.connection_gauge.inc();
        self.connection_total.inc();
        self.global_connection_total.inc();
        MetricsWrappedConnection {
            client: Some(client),
            metrics: self.clone(),
            armed: true,
        }
    }

    pub fn new(
        service: &str,
        service_type: &str,
        pool: &str,
        source: &str,
        provider_name: &Option<String>,
        site_name: &str,
    ) -> Self {
        // Since these metrics live in a pruning registry, we want to take an extra
        // step to pin these global counters into the register. We do that by holding
        // on to them for the life of the program.
        // We do this only for the service_type counters created here, as those have
        // low cardinality (a couple) vs. the potentially unbounded number of service
        // counters that we might create.
        struct GlobalMetrics {
            global_connection_gauge: AtomicCounter,
            global_connection_total: AtomicCounter,
            global_ready_count: AtomicCounter,
            global_msgs_delivered: AtomicCounter,
            global_msgs_transfail: AtomicCounter,
            global_msgs_fail: AtomicCounter,
        }

        static GLOBALS: LazyLock<Mutex<HashMap<String, Arc<GlobalMetrics>>>> =
            LazyLock::new(Mutex::default);

        let globals = {
            let mut g = GLOBALS.lock();
            match g.get(service_type) {
                Some(metrics) => Arc::clone(metrics),
                None => {
                    let metrics = Arc::new(GlobalMetrics {
                        global_connection_gauge:
                            crate::metrics_helper::connection_gauge_for_service(service_type),
                        global_connection_total:
                            crate::metrics_helper::connection_total_for_service(service_type),
                        global_ready_count: crate::metrics_helper::ready_count_gauge_for_service(
                            service_type,
                        ),
                        global_msgs_delivered:
                            crate::metrics_helper::total_msgs_delivered_for_service(service_type),
                        global_msgs_transfail:
                            crate::metrics_helper::total_msgs_transfail_for_service(service_type),
                        global_msgs_fail: crate::metrics_helper::total_msgs_fail_for_service(
                            service_type,
                        ),
                    });
                    g.insert(service_type.to_string(), Arc::clone(&metrics));
                    metrics
                }
            }
        };

        let provider = provider_name
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or(site_name);

        let provider_pool_key = BorrowedProviderAndPoolKey { provider, pool };
        let provider_source_key = BorrowedProviderAndSourceKey {
            provider,
            source,
            pool,
        };
        let provider_key = BorrowedProviderKey { provider };

        let source_provider_msgs_fail = TOTAL_MSGS_FAIL_BY_PROVIDER_AND_SOURCE
            .get_or_create(&provider_source_key as &dyn ProviderAndSourceKeyTrait);
        let source_provider_msgs_delivered = TOTAL_MSGS_DELIVERED_BY_PROVIDER_AND_SOURCE
            .get_or_create(&provider_source_key as &dyn ProviderAndSourceKeyTrait);
        let source_provider_msgs_transfail = TOTAL_MSGS_TRANSFAIL_BY_PROVIDER_AND_SOURCE
            .get_or_create(&provider_source_key as &dyn ProviderAndSourceKeyTrait);

        let provider_msgs_fail =
            TOTAL_MSGS_FAIL_BY_PROVIDER.get_or_create(&provider_key as &dyn ProviderKeyTrait);
        let provider_msgs_delivered =
            TOTAL_MSGS_DELIVERED_BY_PROVIDER.get_or_create(&provider_key as &dyn ProviderKeyTrait);
        let provider_msgs_transfail =
            TOTAL_MSGS_TRANSFAIL_BY_PROVIDER.get_or_create(&provider_key as &dyn ProviderKeyTrait);

        let ready_count = ReadyCountBundle {
            ready_count_by_service: crate::metrics_helper::ready_count_gauge_for_service(&service),
            global_ready_count: globals.global_ready_count.clone(),
            queued_by_provider: QUEUED_COUNT_GAUGE_BY_PROVIDER
                .get_or_create(&provider_key as &dyn ProviderKeyTrait),
            queued_by_provider_and_pool: QUEUED_COUNT_GAUGE_BY_PROVIDER_AND_POOL
                .get_or_create(&provider_pool_key as &dyn ProviderAndPoolKeyTrait),
        };

        let delivered = DispositionBundle {
            msgs: crate::metrics_helper::total_msgs_delivered_for_service(&service),
            global: globals.global_msgs_delivered.clone(),
            provider: provider_msgs_delivered,
            source_provider: source_provider_msgs_delivered,
        };

        let transfail = DispositionBundle {
            msgs: crate::metrics_helper::total_msgs_transfail_for_service(&service),
            global: globals.global_msgs_transfail.clone(),
            provider: provider_msgs_transfail,
            source_provider: source_provider_msgs_transfail,
        };

        let fail = DispositionBundle {
            msgs: crate::metrics_helper::total_msgs_fail_for_service(&service),
            global: globals.global_msgs_fail.clone(),
            provider: provider_msgs_fail,
            source_provider: source_provider_msgs_fail,
        };

        let connection_gauge = ConnectionGaugeBundle {
            connections: crate::metrics_helper::connection_gauge_for_service(&service),
            global: globals.global_connection_gauge.clone(),
            provider: crate::metrics_helper::CONN_GAUGE_BY_PROVIDER
                .get_or_create(&provider_key as &dyn ProviderKeyTrait),
            provider_and_pool: crate::metrics_helper::CONN_GAUGE_BY_PROVIDER_AND_POOL
                .get_or_create(&provider_pool_key as &dyn ProviderAndPoolKeyTrait),
        };

        DeliveryMetrics {
            connection_gauge,
            connection_total: crate::metrics_helper::connection_total_for_service(&service),
            global_connection_total: globals.global_connection_total.clone(),
            ready_full: crate::metrics_helper::ready_full_counter_for_service(&service),
            ready_count,
            deliver_message_rollup: crate::metrics_helper::deliver_message_rollup_for_service(
                service_type,
            ),
            delivered,
            transfail,
            fail,
        }
    }

    pub fn inc_transfail(&self) {
        self.transfail.inc();
    }

    pub fn inc_transfail_by(&self, amount: usize) {
        self.transfail.inc_by(amount);
    }

    pub fn inc_fail(&self) {
        self.fail.inc();
    }

    pub fn inc_fail_by(&self, amount: usize) {
        self.fail.inc_by(amount);
    }

    pub fn inc_delivered(&self) {
        self.delivered.inc();
    }
}

/// A helper struct to manage the number of connections.
/// It increments counters when created by DeliveryMetrics::wrap_connection
/// and decrements them when dropped
#[derive(Debug)]
pub struct MetricsWrappedConnection<T> {
    client: Option<T>,
    metrics: DeliveryMetrics,
    armed: bool,
}

impl<T> MetricsWrappedConnection<T> {
    /// Propagate the count from one type of connection to another
    pub fn map_connection<O>(mut self, client: O) -> MetricsWrappedConnection<O> {
        self.armed = false;
        MetricsWrappedConnection {
            client: Some(client),
            metrics: self.metrics.clone(),
            armed: true,
        }
    }

    pub fn take(mut self) -> T {
        if self.armed {
            self.metrics.connection_gauge.dec();
            self.armed = false;
        }
        self.client.take().expect("to take only once")
    }
}

impl<T> Drop for MetricsWrappedConnection<T> {
    fn drop(&mut self) {
        if self.armed {
            self.metrics.connection_gauge.dec();
        }
    }
}

impl<T> std::ops::Deref for MetricsWrappedConnection<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.client.as_ref().expect("to be valid")
    }
}

impl<T> std::ops::DerefMut for MetricsWrappedConnection<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.client.as_mut().expect("to be valid")
    }
}
