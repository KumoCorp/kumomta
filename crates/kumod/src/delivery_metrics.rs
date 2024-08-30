use kumo_prometheus::AtomicCounter;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use prometheus::Histogram;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct DeliveryMetrics {
    connection_gauge: AtomicCounter,
    global_connection_gauge: AtomicCounter,
    connection_total: AtomicCounter,
    global_connection_total: AtomicCounter,

    pub ready_count: AtomicCounter,
    pub global_ready_count: AtomicCounter,
    pub ready_full: AtomicCounter,

    msgs_delivered: AtomicCounter,
    global_msgs_delivered: AtomicCounter,

    msgs_transfail: AtomicCounter,
    global_msgs_transfail: AtomicCounter,

    msgs_fail: AtomicCounter,
    global_msgs_fail: AtomicCounter,

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
        self.global_connection_gauge.inc();
        self.connection_total.inc();
        self.global_connection_total.inc();
        MetricsWrappedConnection {
            client: Some(client),
            metrics: self.clone(),
            armed: true,
        }
    }

    pub fn new(service: &str, service_type: &str) -> Self {
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

        static GLOBALS: Lazy<Mutex<HashMap<String, Arc<GlobalMetrics>>>> =
            Lazy::new(|| Mutex::new(HashMap::new()));

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

        DeliveryMetrics {
            connection_gauge: crate::metrics_helper::connection_gauge_for_service(&service),
            global_connection_gauge: globals.global_connection_gauge.clone(),
            connection_total: crate::metrics_helper::connection_total_for_service(&service),
            global_connection_total: globals.global_connection_total.clone(),
            ready_full: crate::metrics_helper::ready_full_counter_for_service(&service),
            ready_count: crate::metrics_helper::ready_count_gauge_for_service(&service),
            global_ready_count: globals.global_ready_count.clone(),
            msgs_delivered: crate::metrics_helper::total_msgs_delivered_for_service(&service),
            global_msgs_delivered: globals.global_msgs_delivered.clone(),
            msgs_transfail: crate::metrics_helper::total_msgs_transfail_for_service(&service),
            global_msgs_transfail: globals.global_msgs_transfail.clone(),
            msgs_fail: crate::metrics_helper::total_msgs_fail_for_service(&service),
            global_msgs_fail: globals.global_msgs_fail.clone(),
            deliver_message_rollup: crate::metrics_helper::deliver_message_rollup_for_service(
                service_type,
            ),
        }
    }

    pub fn inc_transfail(&self) {
        self.msgs_transfail.inc();
        self.global_msgs_transfail.inc();
    }

    pub fn inc_transfail_by(&self, amount: usize) {
        self.msgs_transfail.inc_by(amount);
        self.global_msgs_transfail.inc_by(amount);
    }

    pub fn inc_fail(&self) {
        self.msgs_fail.inc();
        self.global_msgs_fail.inc();
    }

    pub fn inc_fail_by(&self, amount: usize) {
        self.msgs_fail.inc_by(amount);
        self.global_msgs_fail.inc_by(amount);
    }

    pub fn inc_delivered(&self) {
        self.msgs_delivered.inc();
        self.global_msgs_delivered.inc();
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
            self.metrics.global_connection_gauge.dec();
            self.armed = false;
        }
        self.client.take().expect("to take only once")
    }
}

impl<T> Drop for MetricsWrappedConnection<T> {
    fn drop(&mut self) {
        if self.armed {
            self.metrics.connection_gauge.dec();
            self.metrics.global_connection_gauge.dec();
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
