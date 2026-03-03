//! Prometheus metrics for KumoProxy
//!
//! This module provides connection tracking and metrics for the proxy server,
//! exposed via a Prometheus-compatible HTTP endpoint.

use kumo_prometheus::{declare_metric, label_key, AtomicCounter};
use std::net::SocketAddr;

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

declare_metric! {
/// Total number of incoming connections accepted by the proxy.
///
/// This counter increments each time a new client connection is accepted
/// by a proxy listener, before any SOCKS5 handshake begins.
pub static TOTAL_CONNECTIONS_ACCEPTED: CounterRegistry<ListenerKey>(
    "proxy_connections_accepted_total"
);
}

declare_metric! {
/// Total number of connections that failed during handshake or proxying.
///
/// This counter increments when a connection fails due to handshake errors,
/// authentication failures, timeouts, or I/O errors during proxying.
pub static TOTAL_CONNECTIONS_FAILED: CounterRegistry<ListenerKey>(
    "proxy_connections_failed_total"
);
}

declare_metric! {
/// Total number of TLS handshake failures.
///
/// This counter increments when TLS is enabled on a listener and
/// the TLS handshake with a client fails.
pub static TOTAL_TLS_HANDSHAKE_FAILURES: CounterRegistry<ListenerKey>(
    "proxy_tls_handshake_failures_total"
);
}

declare_metric! {
/// Total number of proxy sessions that completed successfully.
///
/// This counter increments when a proxy session completes without error,
/// meaning the client connected, was proxied to the destination, and
/// both sides closed cleanly.
pub static TOTAL_CONNECTIONS_COMPLETED: CounterRegistry<ListenerKey>(
    "proxy_connections_completed_total"
);
}

declare_metric! {
/// Current number of active proxy connections.
///
/// This gauge shows the number of connections currently being proxied.
/// It increments when a connection is accepted and decrements when
/// the connection closes (successfully or with error).
pub static ACTIVE_CONNECTIONS: PruningGaugeRegistry<ListenerKey>(
    "proxy_active_connections"
);
}

declare_metric! {
/// Total bytes transferred from client to destination.
///
/// This counter tracks the total number of bytes flowing from proxy clients
/// to their intended destinations (upstream direction).
pub static BYTES_CLIENT_TO_DEST: CounterRegistry<ListenerKey>(
    "proxy_bytes_client_to_dest_total"
);
}

declare_metric! {
/// Total bytes transferred from destination to client.
///
/// This counter tracks the total number of bytes flowing from destinations
/// back to proxy clients (downstream direction).
pub static BYTES_DEST_TO_CLIENT: CounterRegistry<ListenerKey>(
    "proxy_bytes_dest_to_client_total"
);
}

declare_metric! {
/// Total number of outbound connections made to destinations.
///
/// This counter tracks connections by destination IP address.
/// Note: This can create high cardinality if your proxy connects to many
/// unique destinations. The metric uses a pruning counter registry to
/// mitigate memory impact.
pub static OUTBOUND_CONNECTIONS_TOTAL: PruningCounterRegistry<ConnectionKey>(
    "proxy_outbound_connections_total"
);
}

pub fn tls_handshake_failures_for_listener(listener: SocketAddr) -> AtomicCounter {
    let listener_str = listener.to_string();
    let key = BorrowedListenerKey {
        listener: &listener_str,
    };
    TOTAL_TLS_HANDSHAKE_FAILURES.get_or_create(&key as &dyn ListenerKeyTrait)
}

pub fn outbound_connections_for(listener: SocketAddr, destination: SocketAddr) -> AtomicCounter {
    let listener_str = listener.to_string();
    let destination_str = destination.to_string();
    let key = BorrowedConnectionKey {
        listener: &listener_str,
        destination: &destination_str,
    };
    OUTBOUND_CONNECTIONS_TOTAL.get_or_create(&key as &dyn ConnectionKeyTrait)
}

/// Metrics bundle for a single proxy session.
/// Uses RAII pattern: active_connections is incremented on creation
/// and decremented on drop.
pub struct ProxySessionMetrics {
    active_connections: AtomicCounter,
    connections_completed: AtomicCounter,
    connections_failed: AtomicCounter,
    bytes_client_to_dest: AtomicCounter,
    bytes_dest_to_client: AtomicCounter,
}

impl ProxySessionMetrics {
    pub fn new(listener: SocketAddr) -> Self {
        // Create a single BorrowedListenerKey and reuse it for all metric lookups
        // to avoid multiple to_string() calls
        let listener_str = listener.to_string();
        let key = BorrowedListenerKey {
            listener: &listener_str,
        };
        let key_trait = &key as &dyn ListenerKeyTrait;

        TOTAL_CONNECTIONS_ACCEPTED.get_or_create(key_trait).inc();

        let active = ACTIVE_CONNECTIONS.get_or_create(key_trait);
        active.inc();

        Self {
            active_connections: active,
            connections_completed: TOTAL_CONNECTIONS_COMPLETED.get_or_create(key_trait),
            connections_failed: TOTAL_CONNECTIONS_FAILED.get_or_create(key_trait),
            bytes_client_to_dest: BYTES_CLIENT_TO_DEST.get_or_create(key_trait),
            bytes_dest_to_client: BYTES_DEST_TO_CLIENT.get_or_create(key_trait),
        }
    }

    pub fn record_bytes(&self, bytes_to_remote: u64, bytes_to_client: u64) {
        self.bytes_client_to_dest.inc_by(bytes_to_remote as usize);
        self.bytes_dest_to_client.inc_by(bytes_to_client as usize);
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
