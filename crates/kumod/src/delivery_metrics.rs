use prometheus::{Histogram, IntCounter, IntGauge};

#[derive(Clone, Debug)]
pub struct DeliveryMetrics {
    connection_gauge: IntGauge,
    global_connection_gauge: IntGauge,
    connection_total: IntCounter,
    global_connection_total: IntCounter,

    pub ready_count: IntGauge,
    pub ready_full: IntCounter,

    msgs_delivered: IntCounter,
    global_msgs_delivered: IntCounter,

    msgs_transfail: IntCounter,
    global_msgs_transfail: IntCounter,

    msgs_fail: IntCounter,
    global_msgs_fail: IntCounter,

    pub deliver_message_rollup: Histogram,
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
        DeliveryMetrics {
            connection_gauge: crate::metrics_helper::connection_gauge_for_service(&service),
            global_connection_gauge: crate::metrics_helper::connection_gauge_for_service(
                service_type,
            ),
            connection_total: crate::metrics_helper::connection_total_for_service(&service),
            global_connection_total: crate::metrics_helper::connection_total_for_service(
                service_type,
            ),
            ready_full: crate::metrics_helper::ready_full_counter_for_service(&service),
            ready_count: crate::metrics_helper::ready_count_gauge_for_service(&service),
            msgs_delivered: crate::metrics_helper::total_msgs_delivered_for_service(&service),
            global_msgs_delivered: crate::metrics_helper::total_msgs_delivered_for_service(
                service_type,
            ),
            msgs_transfail: crate::metrics_helper::total_msgs_transfail_for_service(&service),
            global_msgs_transfail: crate::metrics_helper::total_msgs_transfail_for_service(
                service_type,
            ),
            msgs_fail: crate::metrics_helper::total_msgs_fail_for_service(&service),
            global_msgs_fail: crate::metrics_helper::total_msgs_fail_for_service(service_type),
            deliver_message_rollup: crate::metrics_helper::deliver_message_rollup_for_service(service_type),
        }
    }

    pub fn inc_transfail(&self) {
        self.msgs_transfail.inc();
        self.global_msgs_transfail.inc();
    }

    pub fn inc_transfail_by(&self, amount: usize) {
        self.msgs_transfail.inc_by(amount as u64);
        self.global_msgs_transfail.inc_by(amount as u64);
    }

    pub fn inc_fail(&self) {
        self.msgs_fail.inc();
        self.global_msgs_fail.inc();
    }

    pub fn inc_fail_by(&self, amount: usize) {
        self.msgs_fail.inc_by(amount as u64);
        self.global_msgs_fail.inc_by(amount as u64);
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
