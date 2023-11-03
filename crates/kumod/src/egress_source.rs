use crate::http_server::admin_suspend_ready_q_v1::AdminSuspendReadyQEntry;
use crate::queue::QueueConfig;
use crate::ready_queue::{ReadyQueueManager, ReadyQueueName};
use anyhow::Context;
use config::{CallbackSignature, LuaConfig};
use gcd::Gcd;
use kumo_server_common::config_handle::ConfigHandle;
use lruttl::LruCacheWithTtl;
use mlua::prelude::LuaUserData;
use serde::{Deserialize, Serialize};
use socksv5::v5::{
    SocksV5AuthMethod, SocksV5Command, SocksV5Host, SocksV5RequestStatus, SocksV5Response,
};
use std::cell::RefCell;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpSocket, TcpStream};

lazy_static::lazy_static! {
    static ref SOURCES: Mutex<LruCacheWithTtl<String, EgressSource>> = Mutex::new(LruCacheWithTtl::new(128));
    static ref POOLS: Mutex<LruCacheWithTtl<String, EgressPool>> = Mutex::new(LruCacheWithTtl::new(128));
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EgressSource {
    /// Give it a friendly name for use in reporting and referencing
    /// elsewhere in the config
    pub name: String,

    /// Specify the ehlo domain that should be used when sending from
    /// this source
    #[serde(default)]
    pub ehlo_domain: Option<String>,

    /// Bind to this local address prior to issuing a connect(2) syscall
    pub source_address: Option<IpAddr>,

    /// Override the default destination port number with this value
    /// for deployments that use port mapping
    pub remote_port: Option<u16>,

    /// The host:port of the haproxy that should be used
    pub ha_proxy_server: Option<SocketAddr>,

    /// Ask ha_proxy to bind to this address when it is making
    /// a connection
    pub ha_proxy_source_address: Option<IpAddr>,

    /// The host:port of the SOCKS5 server that should be used
    pub socks5_proxy_server: Option<SocketAddr>,

    /// Ask the SOCKS5 proxy to bind to this address when it is making
    /// a connection
    pub socks5_proxy_source_address: Option<IpAddr>,

    #[serde(default = "default_ttl", with = "humantime_serde")]
    pub ttl: Duration,
}

impl LuaUserData for EgressSource {}

impl EgressSource {
    pub async fn resolve(name: &str, config: &mut LuaConfig) -> anyhow::Result<Self> {
        if let Some(source) = SOURCES.lock().unwrap().get(name) {
            return Ok(source.clone());
        }

        let source: Self = if name == "unspecified" {
            Self {
                name: name.to_string(),
                ehlo_domain: None,
                ttl: default_ttl(),
                ha_proxy_server: None,
                ha_proxy_source_address: None,
                remote_port: None,
                socks5_proxy_server: None,
                socks5_proxy_source_address: None,
                source_address: None,
            }
        } else {
            let sig = CallbackSignature::<String, EgressSource>::new("get_egress_source");
            config
                .async_call_callback_non_default(&sig, name.to_string())
                .await
                .with_context(|| format!("get_egress_source '{name}'"))?
        };

        SOURCES.lock().unwrap().insert(
            name.to_string(),
            source.clone(),
            Instant::now() + source.ttl,
        );

        Ok(source)
    }

    fn resolve_proxy_protocol(&self, address: SocketAddr) -> anyhow::Result<ProxyProto> {
        use ppp::v2::{Addresses, IPv4, IPv6};
        let source_name = &self.name;

        match (self.ha_proxy_server, self.ha_proxy_source_address) {
            (Some(server), Some(source)) => match (source, address) {
                (IpAddr::V4(src_ip), SocketAddr::V4(dest_ip)) => {
                    return Ok(ProxyProto::HA {
                        server,
                        source,
                        addresses: Addresses::IPv4(IPv4::new(
                            src_ip,
                            *dest_ip.ip(),
                            0,
                            dest_ip.port(),
                        )),
                    })
                }
                (IpAddr::V6(src_ip), SocketAddr::V6(dest_ip)) => {
                    return Ok(ProxyProto::HA {
                        server,
                        source,
                        addresses: Addresses::IPv6(IPv6::new(
                            src_ip,
                            *dest_ip.ip(),
                            0,
                            dest_ip.port(),
                        )),
                    })
                }
                _ => anyhow::bail!(
                    "Skipping {source_name} because \
                     ha_proxy_source_address address family does \
                     not match the destination address family"
                ),
            },
            _ => {}
        };

        match (self.socks5_proxy_server, self.socks5_proxy_source_address) {
            (Some(server), Some(source)) => match (source, address) {
                (IpAddr::V6(_), SocketAddr::V6(_)) | (IpAddr::V4(_), SocketAddr::V4(_)) => {
                    return Ok(ProxyProto::Socks5 {
                        server,
                        source,
                        destination: address,
                    })
                }
                _ => anyhow::bail!(
                    "Skipping {source_name} because \
                     socks5_proxy_source_address address family does \
                     not match the destination address family"
                ),
            },
            _ => Ok(ProxyProto::None),
        }
    }

    pub async fn connect_to(&self, address: SocketAddr) -> anyhow::Result<(TcpStream, SocketAddr)> {
        let source_name = &self.name;

        let proxy_proto = self.resolve_proxy_protocol(address)?;
        let transport_address = proxy_proto.transport_address(address);

        tracing::trace!("will connect {address:?} {transport_address:?} {proxy_proto:?}");

        let socket = match transport_address {
            SocketAddr::V4(_) => TcpSocket::new_v4(),
            SocketAddr::V6(_) => TcpSocket::new_v6(),
        }
        .with_context(|| format!("make socket to connect to {transport_address:?}"))?;

        // No need for Nagle with SMTP request/response
        socket.set_nodelay(true)?;

        if let Some(source) = self.source_address {
            if let Err(err) = socket.bind(SocketAddr::new(source, 0)) {
                // Always log failure to bind: it indicates a critical
                // misconfiguration issue
                let error = format!(
                    "bind {source:?} for source:{source_name} failed: {err:#} \
                    while attempting to connect to {transport_address:?}"
                );
                tracing::error!("{error}");
                anyhow::bail!("{error}");
            }
        }
        let mut stream = socket
            .connect(transport_address)
            .await
            .with_context(|| format!("connect to {transport_address:?}"))?;

        let source_address = proxy_proto
            .perform_handshake(&mut stream, &source_name)
            .await?;

        Ok((stream, source_address))
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EgressPoolEntry {
    /// Name of an EgressSource to include in this pool
    pub name: String,

    /// Weight of this entry in the pool. If all entries have the same
    /// weight, then they have equal chance of being selected.
    /// If one entry has a weight that is twice that of the other
    /// entry in the pool, then it is twice as likely to be selected
    /// as the other one.
    ///
    /// A weight of 0 prevents this entry from being used.
    #[serde(default = "EgressPoolEntry::default_weight")]
    pub weight: u32,
}

impl EgressPoolEntry {
    fn default_weight() -> u32 {
        1
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EgressPool {
    /// Name of the pool
    pub name: String,

    /// and the sources the constitute this pool
    pub entries: Vec<EgressPoolEntry>,

    #[serde(default = "default_ttl", with = "humantime_serde")]
    pub ttl: Duration,
}

impl LuaUserData for EgressPool {}

impl EgressPool {
    pub async fn resolve(name: Option<&str>, config: &mut LuaConfig) -> anyhow::Result<Self> {
        let name = name.unwrap_or("unspecified");

        if let Some(pool) = POOLS.lock().unwrap().get(name) {
            return Ok(pool.clone());
        }

        let pool: Self = if name == "unspecified" {
            Self {
                name: "unspecified".to_string(),
                entries: vec![EgressPoolEntry {
                    name: "unspecified".to_string(),
                    weight: 1,
                }],
                ttl: default_ttl(),
            }
        } else {
            let sig = CallbackSignature::<String, EgressPool>::new("get_egress_pool");

            config
                .async_call_callback_non_default(&sig, name.to_string())
                .await
                .with_context(|| format!("resolving egress pool '{name}'"))?
        };

        // Validate each of the sources
        for entry in &pool.entries {
            EgressSource::resolve(&entry.name, config)
                .await
                .with_context(|| format!("resolving egress pool '{name}'"))?;
        }

        POOLS
            .lock()
            .unwrap()
            .insert(name.to_string(), pool.clone(), Instant::now() + pool.ttl);

        Ok(pool)
    }
}

/// Maintains the state to manage Weighted Round Robin
/// <http://kb.linuxvirtualserver.org/wiki/Weighted_Round-Robin_Scheduling>
#[derive(Debug)]
pub struct EgressPoolRoundRobin {
    pub name: String,
    entries: Vec<EgressPoolEntry>,

    current_index: RefCell<usize>,
    current_weight: RefCell<u32>,
}

#[derive(Debug, Clone)]
pub enum RoundRobinResult {
    /// Use the source with this name
    Source(String),
    /// All pathways are suspended. The smallest time until one
    /// of them is enabled is this delay
    Delay(chrono::Duration),
    /// No sources are configured, or all sources have zero weight
    NoSources,
}

impl EgressPoolRoundRobin {
    pub fn new(pool: &EgressPool) -> Self {
        let mut entries = vec![];

        for entry in &pool.entries {
            if entry.weight == 0 {
                continue;
            }
            entries.push(entry.clone());
        }

        Self {
            name: pool.name.to_string(),
            entries,
            current_index: RefCell::new(0),
            current_weight: RefCell::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    fn next_ignoring_suspend(&self) -> Option<String> {
        self.next_impl(&self.entries)
    }

    fn next_impl(&self, entries: &[EgressPoolEntry]) -> Option<String> {
        if entries.is_empty() {
            return None;
        }

        if entries.len() == 1 {
            return entries.get(0).map(|entry| entry.name.to_string());
        }

        let mut max_weight = 0;
        let mut gcd = 0;
        for entry in entries {
            max_weight = max_weight.max(entry.weight);
            gcd = gcd.gcd(entry.weight);
        }

        if max_weight == 0 {
            return None;
        }

        let mut current_index = self.current_index.borrow_mut();
        let mut current_weight = self.current_weight.borrow_mut();

        loop {
            *current_index = (*current_index + 1) % entries.len();
            if *current_index == 0 {
                *current_weight = current_weight.saturating_sub(gcd);
                if *current_weight == 0 {
                    *current_weight = max_weight;
                }
            }

            if let Some(entry) = entries.get(*current_index) {
                if entry.weight >= *current_weight {
                    return Some(entry.name.to_string());
                }
            }
        }
    }

    pub async fn next(
        &self,
        queue_name: &str,
        queue_config: &ConfigHandle<QueueConfig>,
    ) -> RoundRobinResult {
        if self.entries.is_empty() {
            return RoundRobinResult::NoSources;
        }

        let mut entries = vec![];
        let mut min_delay = None;

        // filter to non-suspended pathways
        for entry in &self.entries {
            if let Ok(ReadyQueueName {
                name: path_name, ..
            }) =
                ReadyQueueManager::compute_queue_name(queue_name, queue_config, &entry.name).await
            {
                match AdminSuspendReadyQEntry::get_for_queue_name(&path_name) {
                    Some(suspend) => {
                        let duration = suspend.get_duration_chrono();
                        min_delay.replace(min_delay.unwrap_or(duration).min(duration));
                    }
                    None => {
                        entries.push(entry.clone());
                    }
                }
            } else {
                // Likely a DNS resolution issue that prevented us from computing
                // the site name to use for the ready queue.
                // We're not in an appropriate context to handle that issue here,
                // but the good news is that without a valid site name, there can't
                // possibly be any suspensions for a ready queue that we can't name
                // so we can continue to populate the entries and pick one.
                // The DNS issue will bubble up almost immediately after we return
                // a source name as our caller will call
                // ReadyQueueManager::resolve_by_queue_name which will surface it.
                entries.push(entry.clone());
            }
        }

        match self.next_impl(&entries) {
            Some(name) => RoundRobinResult::Source(name),
            None => match min_delay {
                Some(duration) => RoundRobinResult::Delay(duration),
                None => RoundRobinResult::NoSources,
            },
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn round_robin() {
        let pool = EgressPool {
            name: "pool".to_string(),
            entries: vec![
                EgressPoolEntry {
                    name: "one".to_string(),
                    weight: 5,
                },
                EgressPoolEntry {
                    name: "two".to_string(),
                    weight: 2,
                },
                EgressPoolEntry {
                    name: "three".to_string(),
                    weight: 3,
                },
            ],
            ttl: default_ttl(),
        };

        let rr = EgressPoolRoundRobin::new(&pool);
        let mut counts = HashMap::new();

        for _ in 0..100 {
            let name = rr.next_ignoring_suspend().unwrap();
            *counts.entry(name).or_insert(0) += 1;
        }

        // Counts should be in the same proportion as the
        // weights of the respective pools
        assert_eq!(counts["one"], 50, "one");
        assert_eq!(counts["two"], 20, "two");
        assert_eq!(counts["three"], 30, "three");
    }
}

#[derive(Debug)]
enum ProxyProto {
    None,
    HA {
        server: SocketAddr,
        addresses: ppp::v2::Addresses,
        source: IpAddr,
    },
    Socks5 {
        server: SocketAddr,
        source: IpAddr,
        destination: SocketAddr,
    },
}

impl ProxyProto {
    fn transport_address(&self, addr: SocketAddr) -> SocketAddr {
        match self {
            Self::Socks5 { server, .. } | Self::HA { server, .. } => *server,
            Self::None => addr,
        }
    }

    /// Setup the proxy connection.
    /// Returns the source address used by the connection;
    /// this is *probably* the external IP, unless your proxy is also
    /// behind some kind of NAT or other topology that obscures its
    /// external IP.
    async fn perform_handshake(
        self,
        mut stream: &mut TcpStream,
        source_name: &str,
    ) -> anyhow::Result<SocketAddr> {
        match self {
            Self::HA {
                addresses, source, ..
            } => {
                use ppp::v2::{Builder, Command, Protocol, Version};
                let header = Builder::with_addresses(
                    Version::Two | Command::Proxy,
                    Protocol::Stream,
                    addresses,
                )
                .build()
                .with_context(|| {
                    format!(
                        "building ha proxy protocol header \
                         for connection from source:{source_name} to {self:?}"
                    )
                })?;

                stream.write_all(&header).await.with_context(|| {
                    format!(
                        "sending ha proxy protocol header \
                         for connection from source:{source_name} to {self:?}"
                    )
                })?;
                Ok((source, 0).into())
            }
            Self::Socks5 {
                source,
                destination,
                ..
            } => {
                socksv5::v5::write_handshake(&mut stream, vec![SocksV5AuthMethod::Noauth]).await?;
                let method = socksv5::v5::read_auth_method(&mut stream).await?;
                if method != SocksV5AuthMethod::Noauth {
                    anyhow::bail!("incompatible SOCKS5 authentication {method:?}");
                }

                let (source_host, source_port) = socket_ip_to_host(source);
                let (dest_host, dest_port) = socket_addr_to_host(destination);

                socksv5::v5::write_request(
                    &mut stream,
                    SocksV5Command::Bind,
                    source_host,
                    source_port,
                )
                .await?;

                let bind_status = socksv5::v5::read_request_status(&mut stream).await?;
                match bind_status.status {
                    SocksV5RequestStatus::Success => {}
                    _ => anyhow::bail!("failed to bind {source:?} via {self:?}: {bind_status:?}"),
                }

                socksv5::v5::write_request(
                    &mut stream,
                    SocksV5Command::Connect,
                    dest_host,
                    dest_port,
                )
                .await?;

                let connect_status = socksv5::v5::read_request_status(&mut stream).await?;

                match connect_status.status {
                    SocksV5RequestStatus::Success => {},
                    _ => anyhow::bail!("failed to connect {source:?} -> {destination} via {self:?}: {connect_status:?}"),
                }

                Ok(socks_response_addr(&connect_status)?)
            }
            Self::None => Ok(stream.local_addr()?),
        }
    }
}

fn socks_response_addr(response: &SocksV5Response) -> std::io::Result<SocketAddr> {
    match &response.host {
        SocksV5Host::Ipv4(ip) => Ok(SocketAddr::new(IpAddr::V4((*ip).into()), response.port)),
        SocksV5Host::Ipv6(ip) => Ok(SocketAddr::new(IpAddr::V6((*ip).into()), response.port)),
        SocksV5Host::Domain(_domain) => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Domain not supported",
        )),
    }
}

fn socket_ip_to_host(addr: IpAddr) -> (SocksV5Host, u16) {
    match addr {
        IpAddr::V4(addr) => (SocksV5Host::Ipv4(addr.octets()), 0),
        IpAddr::V6(addr) => (SocksV5Host::Ipv6(addr.octets()), 0),
    }
}

fn socket_addr_to_host(addr: SocketAddr) -> (SocksV5Host, u16) {
    match addr {
        SocketAddr::V4(addr) => (SocksV5Host::Ipv4(addr.ip().octets()), addr.port()),
        SocketAddr::V6(addr) => (SocksV5Host::Ipv6(addr.ip().octets()), addr.port()),
    }
}

fn default_ttl() -> Duration {
    Duration::from_secs(60)
}
