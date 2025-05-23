use crate::http_server::admin_suspend_ready_q_v1::AdminSuspendReadyQEntry;
use crate::queue::{opt_timeout_at, QueueConfig, ReadyQueueFull};
use crate::ready_queue::{ReadyQueueHandle, ReadyQueueManager, ReadyQueueName};
use anyhow::Context;
use config::epoch::ConfigEpoch;
use config::{CallbackSignature, LuaConfig};
use data_loader::KeySource;
use gcd::Gcd;
use kumo_log_types::MaybeProxiedSourceAddress;
use kumo_server_common::config_handle::ConfigHandle;
use lruttl::declare_cache;
use message::Message;
use mlua::prelude::LuaUserData;
use parking_lot::FairMutex as Mutex;
use prometheus::IntCounter;
use serde::{Deserialize, Serialize};
use socksv5::v5::{
    SocksV5AuthMethod, SocksV5Command, SocksV5Host, SocksV5RequestStatus, SocksV5Response,
};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};

declare_cache! {
static SOURCES: LruCacheWithTtl<String, EgressSource>::new("egress_source_sources", 128);
}
declare_cache! {
static POOLS: LruCacheWithTtl<String, EgressPool>::new("egress_source_pools", 128);
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, mlua::FromLua)]
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

    pub socks5_proxy_username: Option<String>,
    pub socks5_proxy_password: Option<KeySource>,

    #[serde(default = "default_ttl", with = "duration_serde")]
    pub ttl: Duration,
}

impl LuaUserData for EgressSource {}

impl EgressSource {
    pub async fn resolve(name: &str, config: &mut LuaConfig) -> anyhow::Result<Self> {
        SOURCES
            .get_or_try_insert(&name.to_string(), |source| source.ttl, async {
                if name == "unspecified" {
                    Ok(EgressSource {
                        name: name.to_string(),
                        ehlo_domain: None,
                        ttl: default_ttl(),
                        ha_proxy_server: None,
                        ha_proxy_source_address: None,
                        remote_port: None,
                        socks5_proxy_server: None,
                        socks5_proxy_source_address: None,
                        socks5_proxy_username: None,
                        socks5_proxy_password: None,
                        source_address: None,
                    })
                } else {
                    let sig = CallbackSignature::<String, EgressSource>::new("get_egress_source");
                    config
                        .async_call_callback_non_default(&sig, name.to_string())
                        .await
                        .with_context(|| format!("get_egress_source '{name}'"))
                }
            })
            .await
            .map_err(|err| anyhow::anyhow!("{err:#}"))
            .map(|lookup| lookup.item)
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
                (source, server) => anyhow::bail!(
                    "Skipping {source_name} because \
                     ha_proxy_source_address {source} address family does \
                     not match the destination address family {server}"
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
                        username_and_password: match (
                            &self.socks5_proxy_username,
                            &self.socks5_proxy_password,
                        ) {
                            (Some(user), Some(pass)) => Some((user, pass)),
                            _ => None,
                        },
                    })
                }
                (source, server) => anyhow::bail!(
                    "Skipping {source_name} because \
                     socks5_proxy_source_address {source} address family does \
                     not match the destination address family {server}"
                ),
            },
            _ => Ok(ProxyProto::None),
        }
    }

    pub async fn connect_to(
        &self,
        address: SocketAddr,
        timeout_duration: Duration,
    ) -> anyhow::Result<(TcpStream, MaybeProxiedSourceAddress)> {
        let source_name = &self.name;

        let proxy_proto = self.resolve_proxy_protocol(address)?;
        let transport_address = proxy_proto.transport_address(address);

        let transport_context = format!("{transport_address:?} {proxy_proto:?}");
        let connect_context =
            format!("{address:?} transport={transport_address:?} proto={proxy_proto:?}");
        tracing::trace!("will connect to {connect_context}");

        let socket = match transport_address {
            SocketAddr::V4(_) => TcpSocket::new_v4(),
            SocketAddr::V6(_) => TcpSocket::new_v6(),
        }
        .with_context(|| format!("make socket to connect to {connect_context}"))?;

        // No need for Nagle with SMTP request/response
        socket.set_nodelay(true)?;

        if let Some(source) = self.source_address {
            if let Err(err) = socket.bind(SocketAddr::new(source, 0)) {
                let error = format!(
                    "bind {source:?} for source:{source_name} failed: {err:#} \
                    while attempting to connect to {connect_context}"
                );
                static FAILED_BIND: LazyLock<IntCounter> = LazyLock::new(|| {
                    prometheus::register_int_counter!(
                        "bind_failures",
                        "how many times that directly binding a source address has failed"
                    )
                    .unwrap()
                });
                FAILED_BIND.inc();
                anyhow::bail!("{error}");
            }
        }

        let deadline = Instant::now() + timeout_duration;
        let is_proxy = proxy_proto.is_proxy();

        let mut stream =
            match tokio::time::timeout_at(deadline.into(), socket.connect(transport_address)).await
            {
                Err(_) => {
                    inc_failed_proxy_connection_attempts(is_proxy);
                    anyhow::bail!(
                        "timeout after {timeout_duration:?} \
                         while connecting to {transport_context}"
                    );
                }
                Ok(Err(err)) => {
                    inc_failed_proxy_connection_attempts(is_proxy);
                    anyhow::bail!("failed to connect to {transport_context}: {err:#}");
                }
                Ok(Ok(stream)) => stream,
            };

        let source_address = tokio::time::timeout_at(
            deadline.into(),
            proxy_proto.perform_handshake(&mut stream, &source_name),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "timeout after {timeout_duration:?} \
                    while performing proxy handshake with {transport_context}"
            )
        })?
        .with_context(|| format!("failed to perform proxy handshake with {transport_context}"))?;

        Ok((stream, source_address))
    }
}

fn inc_failed_proxy_connection_attempts(is_proxy: bool) {
    if !is_proxy {
        return;
    }

    static FAILED: LazyLock<IntCounter> = LazyLock::new(|| {
        prometheus::register_int_counter!(
            "proxy_connection_failures",
            "how many times a connection attempt to a proxy server has failed"
        )
        .unwrap()
    });
    FAILED.inc();
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, mlua::FromLua)]
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

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, mlua::FromLua)]
#[serde(deny_unknown_fields)]
pub struct EgressPool {
    /// Name of the pool
    pub name: String,

    /// and the sources the constitute this pool
    #[serde(default)]
    pub entries: Vec<EgressPoolEntry>,

    #[serde(default = "default_ttl", with = "duration_serde")]
    pub ttl: Duration,
}

impl LuaUserData for EgressPool {}

impl EgressPool {
    pub async fn resolve(name: Option<&str>, config: &mut LuaConfig) -> anyhow::Result<Self> {
        let name = name.unwrap_or("unspecified");

        let pool = POOLS
            .get_or_try_insert(&name.to_string(), |pool| pool.ttl, async {
                let pool = if name == "unspecified" {
                    EgressPool {
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

                Ok::<_, anyhow::Error>(pool)
            })
            .await
            .map_err(|err: Arc<anyhow::Error>| anyhow::anyhow!("{err:#}"))?
            .item;

        // Validate each of the sources
        for entry in &pool.entries {
            EgressSource::resolve(&entry.name, config)
                .await
                .with_context(|| format!("resolving egress pool '{name}'"))?;
        }
        Ok(pool)
    }
}

/// Maintains the state to manage Weighted Round Robin
/// <http://kb.linuxvirtualserver.org/wiki/Weighted_Round-Robin_Scheduling>
#[derive(Debug)]
pub struct EgressPoolSourceSelector {
    pub name: String,
    entries: Vec<EgressPoolEntry>,

    index_and_weight: Mutex<IndexAndWeight>,
    ready_queue_names: Mutex<HashMap<String, CachedReadyQueueNameEntry>>,
}

#[derive(Clone, Debug)]
enum CachedReadyQueueNameEntry {
    Hit(Arc<CachedReadyQueueName>),
    Negative { expires_at: Instant },
}

#[derive(Debug)]
pub struct CachedReadyQueueName {
    pub name: ReadyQueueName,
    /// queue_config.generation()
    generation: usize,
}

#[derive(Debug)]
struct IndexAndWeight {
    current_index: usize,
    current_weight: u32,
}

#[derive(Debug, Clone)]
pub enum SourceInsertResult {
    /// We inserted it ok!
    Inserted,
    /// QueueManager failed to resolve the named queue
    FailedResolve(String),
    /// All pathways are suspended. The smallest time until one
    /// of them is enabled is this delay
    Delay(chrono::Duration),
    /// No sources are configured, or all sources have zero weight
    NoSources,
}

impl EgressPoolSourceSelector {
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
            index_and_weight: Mutex::new(IndexAndWeight {
                current_index: 0,
                current_weight: 0,
            }),
            ready_queue_names: Mutex::new(HashMap::new()),
        }
    }

    /// Helper to test whether we need to create a new state
    /// to track either a changed pool name or set of sources
    /// in the pool
    pub fn equivalent(&self, pool: &EgressPool) -> bool {
        self.name == pool.name && self.entries == pool.entries
    }

    fn get_ready_queue_for_source(
        &self,
        queue_config: &ConfigHandle<QueueConfig>,
        source: &str,
    ) -> Option<CachedReadyQueueNameEntry> {
        let mut ready_queue_names = self.ready_queue_names.lock();
        let entry = ready_queue_names.get(source)?;

        match entry {
            CachedReadyQueueNameEntry::Hit(name) => {
                if queue_config.generation() != name.generation || name.name.has_expired() {
                    ready_queue_names.remove(source);
                    return None;
                }
            }
            CachedReadyQueueNameEntry::Negative { expires_at } => {
                if Instant::now() >= *expires_at {
                    ready_queue_names.remove(source);
                    return None;
                }
            }
        }
        Some(entry.clone())
    }

    async fn compute_ready_queue_name(
        &self,
        deadline: Option<Instant>,
        queue_name: &str,
        queue_config: &ConfigHandle<QueueConfig>,
        source: &str,
    ) -> CachedReadyQueueNameEntry {
        if let Some(entry) = self.get_ready_queue_for_source(queue_config, source) {
            return entry;
        }

        let generation = queue_config.generation();

        let entry = match opt_timeout_at(
            deadline,
            ReadyQueueManager::compute_queue_name(queue_name, queue_config, source),
        )
        .await
        {
            Ok(name) => {
                CachedReadyQueueNameEntry::Hit(Arc::new(CachedReadyQueueName { name, generation }))
            }
            Err(_) => CachedReadyQueueNameEntry::Negative {
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        };

        self.ready_queue_names
            .lock()
            .insert(source.to_string(), entry.clone());

        entry
    }

    #[cfg(test)]
    fn next_ignoring_suspend(&self) -> Option<String> {
        let entries = self
            .entries
            .iter()
            .map(|entry| (entry.clone(), None))
            .collect::<Vec<_>>();
        self.next_impl(&entries)
            .map(|(entry, _ready_queue_name)| entry)
    }

    fn next_impl(
        &self,
        entries: &[(EgressPoolEntry, Option<Arc<CachedReadyQueueName>>)],
    ) -> Option<(String, Option<Arc<CachedReadyQueueName>>)> {
        if entries.is_empty() {
            return None;
        }

        if entries.len() == 1 {
            return entries.first().map(|(entry, ready_queue_name)| {
                (entry.name.to_string(), ready_queue_name.clone())
            });
        }

        let mut max_weight = 0;
        let mut gcd = 0;
        for (entry, _ready_queue_name) in entries {
            max_weight = max_weight.max(entry.weight);
            gcd = gcd.gcd(entry.weight);
        }

        if max_weight == 0 {
            return None;
        }

        let mut iaw = self.index_and_weight.lock();

        loop {
            iaw.current_index = (iaw.current_index + 1) % entries.len();
            if iaw.current_index == 0 {
                iaw.current_weight = iaw.current_weight.saturating_sub(gcd);
                if iaw.current_weight == 0 {
                    iaw.current_weight = max_weight;
                }
            }

            if let Some((entry, ready_queue_name)) = entries.get(iaw.current_index) {
                if entry.weight >= iaw.current_weight {
                    return Some((entry.name.to_string(), ready_queue_name.clone()));
                }
            }
        }
    }

    pub async fn select_and_insert(
        &self,
        queue_name: &str,
        queue_config: &ConfigHandle<QueueConfig>,
        msg: Message,
        epoch: ConfigEpoch,
        deadline: Option<Instant>,
    ) -> anyhow::Result<SourceInsertResult> {
        if self.entries.is_empty() {
            return Ok(SourceInsertResult::NoSources);
        }

        let mut entries = vec![];
        let mut min_delay = None;
        let mut is_full = false;

        // filter to non-suspended pathways
        for entry in &self.entries {
            match self
                .compute_ready_queue_name(deadline, queue_name, queue_config, &entry.name)
                .await
            {
                CachedReadyQueueNameEntry::Hit(ready_name) => {
                    match AdminSuspendReadyQEntry::get_for_queue_name(&ready_name.name.name) {
                        Some(suspend) => {
                            let duration = suspend.get_duration_chrono();
                            min_delay.replace(min_delay.unwrap_or(duration).min(duration));
                        }
                        None => {
                            entries.push((entry.clone(), Some(ready_name)));
                        }
                    }
                }
                CachedReadyQueueNameEntry::Negative { .. } => {
                    // Likely a DNS resolution issue that prevented us from computing
                    // the site name to use for the ready queue.
                    // We're not in an appropriate context to handle that issue here,
                    // but the good news is that without a valid site name, there can't
                    // possibly be any suspensions for a ready queue that we can't name
                    // so we can continue to populate the entries and pick one.
                    // The DNS issue will bubble up almost immediately after we return
                    // a source name as our caller will call
                    // ReadyQueueManager::resolve_by_queue_name which will surface it.
                    entries.push((entry.clone(), None));
                }
            }
        }

        loop {
            match self.next_impl(&entries) {
                Some((source_name, ready_queue_name)) => {
                    match resolve_queue(
                        ready_queue_name,
                        queue_name,
                        queue_config,
                        &source_name,
                        &self.name,
                        epoch,
                        deadline,
                    )
                    .await
                    {
                        Ok(site) => {
                            match site.make_reservation() {
                                Some(reservation) => {
                                    match get_source_selection_throttle_delay(
                                        deadline,
                                        &site,
                                        &source_name,
                                    )
                                    .await?
                                    {
                                        None => {
                                            site.redeem_reservation(msg, reservation).await;
                                            return Ok(SourceInsertResult::Inserted);
                                        }
                                        Some(delay) => {
                                            // Throttled; revise min delay to match throttle
                                            if let Ok(delay) = chrono::Duration::from_std(delay) {
                                                min_delay
                                                    .replace(min_delay.unwrap_or(delay).min(delay));
                                            }

                                            // fall through
                                        }
                                    }
                                }
                                None => {
                                    // Not usable; it is too full fall through.
                                    is_full = true;
                                }
                            }
                        }
                        Err(err) => {
                            return Ok(SourceInsertResult::FailedResolve(format!("{err:#}")));
                        }
                    };

                    // If we get here, the selected source was not
                    // eligible for use.
                    // Let's try to find another source that has room,
                    // by going around again once we've filtered this
                    // particular source out of the set
                    entries.retain(|(entry, _)| entry.name != source_name);
                }
                None => {
                    // There are no more sources left to consider.
                    //
                    // If we definitively hit a full queue as one
                    // of the candidates, let's return that we are
                    // full
                    return if is_full {
                        Err(ReadyQueueFull.into())
                    } else {
                        // If we got a delay value, it means that at least one
                        // of the candidates was either suspended until that duration,
                        // or was subject to a source_selection_rate with a duration.
                        // Let our response reflect that delay.
                        Ok(match min_delay {
                            Some(duration) => SourceInsertResult::Delay(duration),
                            None => SourceInsertResult::NoSources,
                        })
                    };
                }
            }
        }
    }
}

/// If selection is throttled, return Some(delay)
async fn get_source_selection_throttle_delay(
    deadline: Option<Instant>,
    site: &ReadyQueueHandle,
    source_name: &str,
) -> anyhow::Result<Option<Duration>> {
    let path_config = site.get_path_config().borrow();

    let mut throttles = Vec::with_capacity(
        if path_config.source_selection_rate.is_some() {
            1
        } else {
            0
        } + path_config.additional_source_selection_rates.len(),
    );

    let rate_name;

    if let Some(throttle) = &path_config.source_selection_rate {
        rate_name = format!(
            "kumomta.source_selection_rate.{}.{source_name}",
            site.name()
        );
        throttles.push((&rate_name, throttle));
    }

    for (key, throttle) in &path_config.additional_source_selection_rates {
        throttles.push((key, throttle));
    }

    if throttles.is_empty() {
        return Ok(None);
    }

    Box::pin(async move {
        // Check throttles from smallest to largest so that we avoid
        // taking up a slot from a larger one only to hit a smaller
        // one and not do anything useful with the larger one
        throttles.sort_by_key(|(_, spec)| {
            ((spec.limit as f64 / spec.period as f64) * 1_000_000.0) as u64
        });

        opt_timeout_at(deadline, async {
            for (key, throttle) in throttles {
                let result = throttle.throttle(&key).await?;
                if let Some(delay) = result.retry_after {
                    return Ok(Some(delay));
                }
            }
            Ok(None)
        })
        .await
    })
    .await
}

async fn resolve_queue(
    ready_queue_name: Option<Arc<CachedReadyQueueName>>,
    queue_name: &str,
    queue_config: &ConfigHandle<QueueConfig>,
    egress_source: &str,
    egress_pool: &str,
    epoch: ConfigEpoch,
    deadline: Option<Instant>,
) -> anyhow::Result<ReadyQueueHandle> {
    if let Some(ready_name) = &ready_queue_name {
        if let Some(site) = ReadyQueueManager::get_by_ready_queue_name(&ready_name.name) {
            return Ok(site);
        }
    }

    opt_timeout_at(
        deadline,
        ReadyQueueManager::resolve_by_queue_name(
            queue_name,
            queue_config,
            egress_source,
            egress_pool,
            epoch,
        ),
    )
    .await
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

        let rr = EgressPoolSourceSelector::new(&pool);
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
enum ProxyProto<'a> {
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
        username_and_password: Option<(&'a str, &'a KeySource)>,
    },
}

impl<'a> ProxyProto<'a> {
    fn transport_address(&self, addr: SocketAddr) -> SocketAddr {
        match self {
            Self::Socks5 { server, .. } | Self::HA { server, .. } => *server,
            Self::None => addr,
        }
    }

    fn is_proxy(&self) -> bool {
        match self {
            Self::None => false,
            _ => true,
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
    ) -> anyhow::Result<MaybeProxiedSourceAddress> {
        match self {
            Self::HA {
                addresses,
                source,
                server,
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
                Ok(MaybeProxiedSourceAddress {
                    address: SocketAddr::from((source, 0)).into(),
                    protocol: Some("haproxy".into()),
                    server: Some(server),
                })
            }
            Self::Socks5 {
                source,
                destination,
                ref username_and_password,
                server,
            } => {
                let mut auth_methods = vec![SocksV5AuthMethod::Noauth];
                if username_and_password.is_some() {
                    auth_methods.push(SocksV5AuthMethod::UsernamePassword);
                }
                socksv5::v5::write_handshake(&mut stream, auth_methods).await?;
                let method = socksv5::v5::read_auth_method(&mut stream).await?;
                match method {
                    SocksV5AuthMethod::Noauth => {}
                    SocksV5AuthMethod::UsernamePassword => {
                        // <https://www.rfc-editor.org/rfc/rfc1929>
                        let (user, pass) = username_and_password.as_ref().ok_or_else(||{
                            anyhow::anyhow!("server responded with UsernamePassword method when we didn't ask for it")
                        })?;

                        let pass = pass
                            .get()
                            .await
                            .context("failed to retrieve socks5 password")?;

                        anyhow::ensure!(
                            user.len() < 256,
                            "username is too long for SOCKS5 protocol"
                        );
                        anyhow::ensure!(
                            pass.len() < 256,
                            "username is too long for SOCKS5 protocol"
                        );

                        {
                            let mut auth_request = vec![];
                            auth_request.push(1); // RFC1929 version 1
                            auth_request.push(user.len() as u8);
                            auth_request.extend_from_slice(user.as_bytes());
                            auth_request.push(pass.len() as u8);
                            auth_request.extend_from_slice(&pass);

                            tracing::debug!("Sending SOCKS5 auth request: {auth_request:#x?}");
                            stream
                                .write_all(&auth_request)
                                .await
                                .context("failed to write SOCKS5 auth request")?;
                        }

                        let mut auth_response_version = [0u8];
                        stream
                            .read_exact(&mut auth_response_version)
                            .await
                            .context("failed to read SOCKS5 auth response (version)")?;
                        anyhow::ensure!(
                            auth_response_version == [1],
                            "invalid SOCKS5 UsernamePassword response packet {auth_response_version:?}"
                        );

                        let mut auth_response_status = [0u8];
                        stream
                            .read_exact(&mut auth_response_status)
                            .await
                            .context("failed to read SOCKS5 auth response (status)")?;

                        anyhow::ensure!(
                            auth_response_status == [0],
                            "SOCKS5 username/password was incorrect"
                        );

                        tracing::debug!("SOCKS5 authentication succeeded!");
                    }
                    _ => {
                        anyhow::bail!("incompatible SOCKS5 authentication {method:?}");
                    }
                }

                let (source_host, source_port) = socket_ip_to_host(source);
                let (dest_host, dest_port) = socket_addr_to_host(destination);

                tracing::debug!("SOCKS5: requesting Bind of {source_host:?}:{source_port}");
                socksv5::v5::write_request(
                    &mut stream,
                    SocksV5Command::Bind,
                    source_host,
                    source_port,
                )
                .await?;

                let bind_status = socksv5::v5::read_request_status(&mut stream).await?;
                match bind_status.status {
                    SocksV5RequestStatus::Success => {
                        tracing::debug!("SOCKS5: bind response: {bind_status:?}");
                    }
                    _ => anyhow::bail!("failed to bind {source:?} via {self:?}: {bind_status:?}"),
                }

                tracing::debug!("SOCKS5: requesting connect to {dest_host:?}:{dest_port}");
                socksv5::v5::write_request(
                    &mut stream,
                    SocksV5Command::Connect,
                    dest_host,
                    dest_port,
                )
                .await?;

                let connect_status = socksv5::v5::read_request_status(&mut stream).await?;

                match connect_status.status {
                    SocksV5RequestStatus::Success => {
                        tracing::debug!("SOCKS5: connected with status {connect_status:?}");
                    },
                    _ => anyhow::bail!("failed to connect {source:?} -> {destination} via {self:?}: {connect_status:?}"),
                }

                Ok(MaybeProxiedSourceAddress {
                    address: socks_response_addr(&connect_status)?.into(),
                    server: Some(server),
                    protocol: Some("socks5".into()),
                })
            }
            Self::None => Ok(MaybeProxiedSourceAddress {
                address: stream.local_addr()?.into(),
                server: None,
                protocol: None,
            }),
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
