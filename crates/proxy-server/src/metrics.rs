//! Prometheus metrics for KumoProxy
//!
//! This module provides connection tracking and metrics for the proxy server,
//! exposed via a Prometheus-compatible HTTP endpoint.

use kumo_prometheus::{label_key, AtomicCounter, CounterRegistry, PruningCounterRegistry};
use std::sync::LazyLock;

// Label key for metrics by listener address
label_key! {
    pub struct ListenerKey {
        pub listener: String,
    }
}

// Label key for metrics by listener and destination
label_key! {
    pub struct ConnectionKey {
        pub listener: String,
        pub destination: String,
    }
}

/// Total number of incoming connections accepted (counter)
pub static TOTAL_CONNECTIONS_ACCEPTED: LazyLock<CounterRegistry<ListenerKey>> =
    LazyLock::new(|| {
        CounterRegistry::register(
            "proxy_connections_accepted_total",
            "total number of incoming connections accepted by the proxy",
        )
    });

/// Total number of connections that failed during handshake or setup (counter)
pub static TOTAL_CONNECTIONS_FAILED: LazyLock<CounterRegistry<ListenerKey>> = LazyLock::new(|| {
    CounterRegistry::register(
        "proxy_connections_failed_total",
        "total number of connections that failed during handshake or proxying",
    )
});

/// Total number of successful proxy sessions completed (counter)
pub static TOTAL_CONNECTIONS_COMPLETED: LazyLock<CounterRegistry<ListenerKey>> =
    LazyLock::new(|| {
        CounterRegistry::register(
            "proxy_connections_completed_total",
            "total number of proxy sessions that completed successfully",
        )
    });

/// Current number of active connections (gauge)
pub static ACTIVE_CONNECTIONS: LazyLock<PruningCounterRegistry<ListenerKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register_gauge(
            "proxy_active_connections",
            "current number of active proxy connections",
        )
    });

/// Total bytes received from clients (counter)
pub static BYTES_RECEIVED: LazyLock<CounterRegistry<ListenerKey>> = LazyLock::new(|| {
    CounterRegistry::register(
        "proxy_bytes_received_total",
        "total bytes received from clients",
    )
});

/// Total bytes sent to clients (counter)
pub static BYTES_SENT: LazyLock<CounterRegistry<ListenerKey>> = LazyLock::new(|| {
    CounterRegistry::register("proxy_bytes_sent_total", "total bytes sent to clients")
});

/// Total outbound connections made to destinations (counter)
pub static OUTBOUND_CONNECTIONS_TOTAL: LazyLock<PruningCounterRegistry<ConnectionKey>> =
    LazyLock::new(|| {
        PruningCounterRegistry::register(
            "proxy_outbound_connections_total",
            "total number of outbound connections made to destinations",
        )
    });

/// Helper to get or create a counter for a given listener
pub fn connections_accepted_for_listener(listener: &str) -> AtomicCounter {
    let key = BorrowedListenerKey { listener };
    TOTAL_CONNECTIONS_ACCEPTED.get_or_create(&key as &dyn ListenerKeyTrait)
}

pub fn connections_failed_for_listener(listener: &str) -> AtomicCounter {
    let key = BorrowedListenerKey { listener };
    TOTAL_CONNECTIONS_FAILED.get_or_create(&key as &dyn ListenerKeyTrait)
}

pub fn connections_completed_for_listener(listener: &str) -> AtomicCounter {
    let key = BorrowedListenerKey { listener };
    TOTAL_CONNECTIONS_COMPLETED.get_or_create(&key as &dyn ListenerKeyTrait)
}

pub fn active_connections_for_listener(listener: &str) -> AtomicCounter {
    let key = BorrowedListenerKey { listener };
    ACTIVE_CONNECTIONS.get_or_create(&key as &dyn ListenerKeyTrait)
}

pub fn bytes_received_for_listener(listener: &str) -> AtomicCounter {
    let key = BorrowedListenerKey { listener };
    BYTES_RECEIVED.get_or_create(&key as &dyn ListenerKeyTrait)
}

pub fn bytes_sent_for_listener(listener: &str) -> AtomicCounter {
    let key = BorrowedListenerKey { listener };
    BYTES_SENT.get_or_create(&key as &dyn ListenerKeyTrait)
}

pub fn outbound_connections_for(listener: &str, destination: &str) -> AtomicCounter {
    let key = BorrowedConnectionKey {
        listener,
        destination,
    };
    OUTBOUND_CONNECTIONS_TOTAL.get_or_create(&key as &dyn ConnectionKeyTrait)
}

/// Metrics bundle for a single proxy session
#[derive(Clone)]
pub struct ProxySessionMetrics {
    #[allow(dead_code)]
    listener: String,
    active_connections: AtomicCounter,
    connections_completed: AtomicCounter,
    connections_failed: AtomicCounter,
    bytes_received: AtomicCounter,
    bytes_sent: AtomicCounter,
}

impl ProxySessionMetrics {
    pub fn new(listener: &str) -> Self {
        let active = active_connections_for_listener(listener);
        active.inc();

        Self {
            listener: listener.to_string(),
            active_connections: active,
            connections_completed: connections_completed_for_listener(listener),
            connections_failed: connections_failed_for_listener(listener),
            bytes_received: bytes_received_for_listener(listener),
            bytes_sent: bytes_sent_for_listener(listener),
        }
    }

    pub fn record_bytes_received(&self, bytes: usize) {
        self.bytes_received.inc_by(bytes);
    }

    pub fn record_bytes_sent(&self, bytes: usize) {
        self.bytes_sent.inc_by(bytes);
    }

    pub fn mark_completed(&self) {
        self.connections_completed.inc();
    }

    pub fn mark_failed(&self) {
        self.connections_failed.inc();
    }
}

impl Drop for ProxySessionMetrics {
    fn drop(&mut self) {
        self.active_connections.dec();
    }
}
