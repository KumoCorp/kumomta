use crate::http_server::admin_suspend_ready_q_v1::AdminSuspendReadyQEntry;
use crate::queue::{opt_timeout_at, QueueConfig, ReadyQueueFull};
use crate::ready_queue::{ReadyQueueHandle, ReadyQueueManager, ReadyQueueName};
use anyhow::Context;
use config::epoch::ConfigEpoch;
use config::{CallbackSignature, LuaConfig};
use data_loader::KeySource;
use dns_resolver::{resolve_socket_addr, IpLookupStrategy};
use gcd::Gcd;
use kumo_address::resolvable::ResolvableSocketAddr;
use kumo_api_types::shaping::Trigger;
use kumo_log_types::MaybeProxiedSourceAddress;
use kumo_prometheus::declare_metric;
use kumo_server_common::config_handle::ConfigHandle;
use lruttl::declare_cache;
use message::Message;
use mlua::prelude::LuaUserData;
use parking_lot::FairMutex as Mutex;
use serde::{Deserialize, Serialize};
use socksv5::v5::{
    SocksV5AuthMethod, SocksV5Command, SocksV5Host, SocksV5RequestStatus, SocksV5Response,
};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};

declare_cache! {
/// Caches EgressSource information by source name
static SOURCES: LruCacheWithTtl<String, EgressSource>::new("egress_source_sources", 128);
}
declare_cache! {
/// Caches EgressPool information by pool name
static POOLS: LruCacheWithTtl<String, EgressPool>::new("egress_source_pools", 128);
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, mlua::FromLua)]
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

    /// The host:port of the haproxy that should be used.
    /// May be an IP literal, or {{since('dev', inline=True)}} a DNS host
    /// name; DNS names are resolved at connection time and each returned
    /// address is tried in turn.
    pub ha_proxy_server: Option<ResolvableSocketAddr>,

    /// Ask ha_proxy to bind to this address when it is making
    /// a connection
    pub ha_proxy_source_address: Option<IpAddr>,

    /// The host:port of the SOCKS5 server that should be used.
    /// May be an IP literal, or {{since('dev', inline=True)}} a DNS host
    /// name; DNS names are resolved at connection time and each returned
    /// address is tried in turn.
    pub socks5_proxy_server: Option<ResolvableSocketAddr>,

    /// Ask the SOCKS5 proxy to bind to this address when it is making
    /// a connection
    pub socks5_proxy_source_address: Option<IpAddr>,

    pub socks5_proxy_username: Option<String>,
    pub socks5_proxy_password: Option<KeySource>,

    /// {{since('dev', inline=True)}} Auto-suspend this source when its
    /// local `source_address` appears to be unplumbed (bind returns
    /// `EADDRNOTAVAIL`). The source is skipped during pool selection
    /// for the configured `duration`; messages assigned to a pool whose
    /// every source is suspended will be delayed until the earliest
    /// suspension expires.
    #[serde(default)]
    pub suspend_when_unplumbed: Option<SuspendOnFailure>,

    /// {{since('dev', inline=True)}} Auto-suspend this source when its
    /// configured proxy server appears unreachable (connect/handshake
    /// failures, or the proxy itself reporting a bind failure for the
    /// requested source address). See [`suspend_when_unplumbed`] for
    /// the suspension semantics.
    #[serde(default)]
    pub suspend_when_proxy_unhealthy: Option<SuspendOnFailure>,

    #[serde(default = "default_ttl", with = "duration_serde")]
    pub ttl: Duration,
}

/// Configuration for an auto-suspend rule on an egress source. The rule
/// fires when its `trigger` condition is met for the relevant failure
/// class, suspending the source from pool selection for `duration`.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, mlua::FromLua)]
#[serde(deny_unknown_fields)]
pub struct SuspendOnFailure {
    /// When to trigger the suspension. Defaults to `Immediate` — a single
    /// matching failure trips the rule. Use `Threshold("N/period")` to
    /// tolerate transient noise (N events in the rolling window before
    /// firing).
    #[serde(default)]
    pub trigger: Trigger,

    /// How long the source stays suspended once the rule fires.
    #[serde(with = "duration_serde")]
    pub duration: Duration,
}

impl LuaUserData for EgressSource {}

#[derive(thiserror::Error, Debug)]
#[error("bind {source_ip:?} for {source_name} failed: {error:#} while attempting to connect to {connect_context}")]
pub struct BindError {
    pub source_ip: IpAddr,
    pub error: std::io::Error,
    pub connect_context: String,
    pub source_name: String,
}

impl BindError {
    pub fn is_unplumbed(&self) -> bool {
        matches!(self.error.kind(), std::io::ErrorKind::AddrNotAvailable)
    }
}

#[derive(thiserror::Error, Debug)]
#[error("{reason}")]
pub struct ConnectError {
    pub is_proxy: bool,
    pub reason: String,
}

#[derive(thiserror::Error, Debug)]
#[error("{reason}")]
pub struct ProxyBindError {
    pub reason: String,
}

pub fn err_match_anyhow<T: std::error::Error + 'static>(err: &anyhow::Error) -> Option<&T> {
    err_match(err.root_cause())
}

pub fn err_match<'a, T: std::error::Error + 'static>(
    err: &'a (dyn std::error::Error + 'static),
) -> Option<&'a T> {
    if let Some(cause) = err.source() {
        err_match(cause)
    } else {
        err.downcast_ref::<T>()
    }
}

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
                        suspend_when_unplumbed: None,
                        suspend_when_proxy_unhealthy: None,
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

    /// Resolve the configured proxy server (if any) into a list of concrete
    /// `ProxyProto` candidates. For a hostname proxy, returns one candidate
    /// per resolved IP. For an IP literal, returns a single candidate. For
    /// no proxy, returns a single `ProxyProto::None`.
    ///
    /// If `self.source_address` is set, candidates are pre-filtered to the
    /// matching IP family so that `bind` cannot fail with a guaranteed
    /// family mismatch.
    async fn resolve_proxy_protocols(
        &'_ self,
        address: SocketAddr,
        deadline: Instant,
    ) -> anyhow::Result<Vec<ProxyProto<'_>>> {
        use ppp::v2::{Addresses, IPv4, IPv6};
        let source_name = &self.name;

        if let (Some(server), Some(source)) =
            (&self.ha_proxy_server, self.ha_proxy_source_address)
        {
            let addresses = match (source, address) {
                (IpAddr::V4(src_ip), SocketAddr::V4(dest_ip)) => {
                    Addresses::IPv4(IPv4::new(src_ip, *dest_ip.ip(), 0, dest_ip.port()))
                }
                (IpAddr::V6(src_ip), SocketAddr::V6(dest_ip)) => {
                    Addresses::IPv6(IPv6::new(src_ip, *dest_ip.ip(), 0, dest_ip.port()))
                }
                (source, _) => anyhow::bail!(
                    "Skipping {source_name} because \
                     ha_proxy_source_address {source} address family does \
                     not match the destination address family {address}"
                ),
            };

            let candidates = self
                .resolve_proxy_candidates(server, deadline, "ha_proxy_server")
                .await?;

            return Ok(candidates
                .into_iter()
                .map(|server_addr| ProxyProto::HA {
                    server: server_addr,
                    source,
                    addresses: addresses.clone(),
                })
                .collect());
        }

        if let (Some(server), Some(source)) =
            (&self.socks5_proxy_server, self.socks5_proxy_source_address)
        {
            match (source, address) {
                (IpAddr::V6(_), SocketAddr::V6(_)) | (IpAddr::V4(_), SocketAddr::V4(_)) => {}
                (source, _) => anyhow::bail!(
                    "Skipping {source_name} because \
                     socks5_proxy_source_address {source} address family does \
                     not match the destination address family {address}"
                ),
            }

            let candidates = self
                .resolve_proxy_candidates(server, deadline, "socks5_proxy_server")
                .await?;

            let username_and_password =
                match (&self.socks5_proxy_username, &self.socks5_proxy_password) {
                    (Some(user), Some(pass)) => Some((user.as_str(), pass)),
                    _ => None,
                };

            return Ok(candidates
                .into_iter()
                .map(|server_addr| ProxyProto::Socks5 {
                    server: server_addr,
                    source,
                    destination: address,
                    username_and_password,
                })
                .collect());
        }

        Ok(vec![ProxyProto::None])
    }

    async fn resolve_proxy_candidates(
        &self,
        server: &ResolvableSocketAddr,
        deadline: Instant,
        field_name: &str,
    ) -> anyhow::Result<Vec<SocketAddr>> {
        let source_name = &self.name;
        let resolved = tokio::time::timeout_at(
            deadline.into(),
            resolve_socket_addr(server, None, IpLookupStrategy::Ipv4AndIpv6),
        )
        .await
        .with_context(|| {
            format!("timeout resolving {field_name} {server} for source {source_name}")
        })?
        .with_context(|| format!("resolving {field_name} {server} for source {source_name}"))?;

        let mut candidates: Vec<SocketAddr> = resolved
            .into_iter()
            .filter_map(|r| r.addr.ip_and_port())
            .collect();

        if let Some(source) = self.source_address {
            let before = candidates.len();
            candidates.retain(|sa| match (source, sa) {
                (IpAddr::V4(_), SocketAddr::V4(_)) | (IpAddr::V6(_), SocketAddr::V6(_)) => true,
                _ => false,
            });
            if candidates.is_empty() {
                anyhow::bail!(
                    "source {source_name}: no {field_name} candidates remain after filtering \
                     {before} resolved address(es) to match source_address {source} family"
                );
            }
        }

        if candidates.is_empty() {
            anyhow::bail!(
                "source {source_name}: {field_name} {server} resolved to no usable addresses"
            );
        }

        Ok(candidates)
    }

    pub async fn connect_to(
        &self,
        address: SocketAddr,
        timeout_duration: Duration,
    ) -> anyhow::Result<(TcpStream, MaybeProxiedSourceAddress)> {
        let source_name = &self.name;

        // Fail fast rather than waiting the full connect timeout.
        if source_health::suspension(source_name).is_some() {
            return Err(ConnectError {
                is_proxy: false,
                reason: format!(
                    "source {source_name} is unhealthy and suspended"
                ),
            }
            .into());
        }

        let deadline = Instant::now() + timeout_duration;

        let candidates = self.resolve_proxy_protocols(address, deadline).await?;

        let mut errors: Vec<(SocketAddr, anyhow::Error)> = Vec::new();
        for proxy in candidates {
            let transport_address = proxy.transport_address(address);
            match self
                .connect_one(address, proxy, deadline, timeout_duration)
                .await
            {
                Ok(result) => return Ok(result),
                Err(err) => errors.push((transport_address, err)),
            }
        }

        let err = combine_connect_errors(source_name, errors);
        self.maybe_record_health_failure(&err);
        Err(err)
    }

    /// Inspect the aggregated connect error and, if this source has the
    /// corresponding `suspend_when_*` rule configured, record the failure
    /// against the rule's counter; the rule may then install a suspension.
    ///
    /// Note: when [`combine_connect_errors`] aggregates a mix of failure
    /// kinds across candidates (e.g. one unplumbed local bind + one plain
    /// connect refused), it returns a typed `ConnectError(is_proxy=true)`
    /// rather than a `BindError`. Such mixed cases therefore count toward
    /// `suspend_when_proxy_unhealthy` rather than `suspend_when_unplumbed`.
    /// Source-family pre-filtering keeps this case rare in practice.
    fn maybe_record_health_failure(&self, err: &anyhow::Error) {
        use source_health::HealthEvent;
        if let Some(b) = err_match_anyhow::<BindError>(err) {
            if b.is_unplumbed() {
                source_health::record(
                    &self.name,
                    HealthEvent::Unplumbed,
                    self.suspend_when_unplumbed.as_ref(),
                );
            }
            return;
        }
        if err_match_anyhow::<ProxyBindError>(err).is_some() {
            source_health::record(
                &self.name,
                HealthEvent::ProxyUnhealthy,
                self.suspend_when_proxy_unhealthy.as_ref(),
            );
            return;
        }
        if let Some(c) = err_match_anyhow::<ConnectError>(err) {
            if c.is_proxy {
                source_health::record(
                    &self.name,
                    HealthEvent::ProxyUnhealthy,
                    self.suspend_when_proxy_unhealthy.as_ref(),
                );
            }
        }
    }

    async fn connect_one(
        &self,
        address: SocketAddr,
        proxy: ProxyProto<'_>,
        deadline: Instant,
        timeout_duration: Duration,
    ) -> anyhow::Result<(TcpStream, MaybeProxiedSourceAddress)> {
        let source_name = &self.name;
        let transport_address = proxy.transport_address(address);
        let is_proxy = proxy.is_proxy();
        let transport_context = format!("{transport_address:?} {proxy:?}");
        let connect_context =
            format!("{address:?} transport={transport_address:?} proto={proxy:?}");
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
                declare_metric! {
                    /// How many times that directly binding a source address has failed.
                    ///
                    /// This generally indicates a configuration error where a source
                    /// is trying to assign an IP address that is not plumbing on
                    /// the system on which kumod is running.
                    static FAILED_BIND: IntCounter("bind_failures");
                }

                FAILED_BIND.inc();
                return Err(BindError {
                    source_ip: source,
                    error: err,
                    connect_context,
                    source_name: source_name.to_string(),
                }
                .into());
            }
        }

        let mut stream =
            match tokio::time::timeout_at(deadline.into(), socket.connect(transport_address)).await
            {
                Err(_) => {
                    inc_failed_proxy_connection_attempts(is_proxy);
                    return Err(ConnectError {
                        is_proxy,
                        reason: format!(
                            "timeout after {timeout_duration:?} \
                             while connecting to {transport_context}"
                        ),
                    }
                    .into());
                }
                Ok(Err(err)) => {
                    inc_failed_proxy_connection_attempts(is_proxy);
                    return Err(ConnectError {
                        is_proxy,
                        reason: format!("failed to connect to {transport_context}: {err:#}"),
                    }
                    .into());
                }
                Ok(Ok(stream)) => stream,
            };

        let source_address = tokio::time::timeout_at(
            deadline.into(),
            proxy.perform_handshake(&mut stream, source_name),
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

/// Combine per-candidate errors from the connect_to loop into a single error.
///
/// If every candidate failed with the same typed error (all `BindError` or
/// all `ProxyBindError`), the first error is returned unchanged so callers
/// that downcast (e.g. `is_unplumbed` classification) keep working. Otherwise
/// the errors are aggregated into a `ConnectError` whose `is_proxy` is true
/// if any failed candidate was a proxy candidate.
fn combine_connect_errors(
    source_name: &str,
    errors: Vec<(SocketAddr, anyhow::Error)>,
) -> anyhow::Error {
    if errors.len() == 1 {
        return errors.into_iter().next().expect("len == 1").1;
    }

    if errors
        .iter()
        .all(|(_, e)| err_match_anyhow::<BindError>(e).is_some())
    {
        return errors.into_iter().next().expect("non-empty").1;
    }
    if errors
        .iter()
        .all(|(_, e)| err_match_anyhow::<ProxyBindError>(e).is_some())
    {
        return errors.into_iter().next().expect("non-empty").1;
    }

    let is_proxy = errors.iter().any(|(_, e)| {
        err_match_anyhow::<ConnectError>(e).is_some_and(|c| c.is_proxy)
            || err_match_anyhow::<ProxyBindError>(e).is_some()
    });
    let detail = errors
        .iter()
        .map(|(addr, err)| format!("{addr}: {err:#}"))
        .collect::<Vec<_>>()
        .join("; ");
    ConnectError {
        is_proxy,
        reason: format!(
            "failed to connect via any of {n} candidate(s) for source {source_name}: {detail}",
            n = errors.len()
        ),
    }
    .into()
}

fn inc_failed_proxy_connection_attempts(is_proxy: bool) {
    if !is_proxy {
        return;
    }

    declare_metric! {
        /// How many times a connection attempt to a proxy server has failed.
        ///
        /// This might indicate either a configuration error (eg: an
        /// incorrect proxy server has been configured) or a
        /// service interruption with that proxy server.
        static FAILED: IntCounter("proxy_connection_failures");
    }
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
            // Cheap early-out: if no source on this node is currently
            // auto-suspended, this is a single relaxed atomic load and
            // a branch — invisible in the hot path.
            if let Some(remaining) = source_health::suspension(&entry.name) {
                let d = chrono::Duration::from_std(remaining)
                    .unwrap_or(kumo_chrono_helper::MINUTE);
                min_delay.replace(min_delay.unwrap_or(d).min(d));
                continue;
            }
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
                    _ => {
                        return Err(ProxyBindError {
                            reason: format!(
                                "The proxy server failed to bind {source:?} \
                                 via {self:?}: {bind_status:?}"
                            ),
                        }
                        .into())
                    }
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

// Source health: per-source auto-suspension on repeated connect-time
// failures. State is process-local and intentionally not coordinated
// across the cluster.
pub(crate) mod source_health {
    use super::SuspendOnFailure;
    use dashmap::{mapref::entry::Entry, DashMap};
    use kumo_api_types::shaping::Trigger;
    use kumo_counter_series::{CounterSeries, CounterSeriesConfig};
    use kumo_prometheus::declare_metric;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{LazyLock, Once};
    use std::time::{Duration, Instant};
    use tokio::sync::Notify;

    /// Which failure class a source-health event belongs to.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum HealthEvent {
        /// The source's own local bind failed because the address is not
        /// plumbed on this host.
        Unplumbed,
        /// The configured proxy server was unreachable, timed out, or
        /// reported a bind failure for the requested source address.
        ProxyUnhealthy,
    }

    impl HealthEvent {
        fn label(self) -> &'static str {
            match self {
                Self::Unplumbed => "Unplumbed",
                Self::ProxyUnhealthy => "ProxyUnhealthy",
            }
        }
    }

    /// Hot path. Reads `SUSPENDED_COUNT` first; in the common case
    /// where no source on this node is suspended, returns `None` after
    /// one relaxed atomic load and a branch.
    pub fn suspension(name: &str) -> Option<Duration> {
        if SUSPENDED_COUNT.load(Ordering::Relaxed) == 0 {
            return None;
        }
        let entry = SUSPENDED.get(name)?;
        let now = Instant::now();
        (entry.expires > now).then(|| entry.expires - now)
    }

    /// Record a classified connection failure against a source.
    ///
    /// Always bumps the per-source per-kind failure counter so operators
    /// see the signal even before they configure auto-suspension. If
    /// `cfg` is `Some` (i.e. the matching `suspend_when_*` field is set
    /// on the source), evaluates the rule's trigger and, on match,
    /// installs a suspension.
    pub fn record(source_name: &str, event: HealthEvent, cfg: Option<&SuspendOnFailure>) {
        FAILURES
            .with_label_values(&[source_name, event.label()])
            .inc();

        let Some(cfg) = cfg else { return };

        let triggered = match &cfg.trigger {
            Trigger::Immediate => true,
            Trigger::Threshold(spec) => {
                let mut entry = COUNTERS.entry(source_name.to_string()).or_default();
                let series = entry.series_for(event, spec.period);
                series.increment(1);
                series.sum() >= spec.limit
            }
        };
        if triggered {
            install_suspension(source_name, event, cfg.duration);
        }
    }

    /// One suspension entry, keyed by source name in `SUSPENDED`.
    #[derive(Copy, Clone)]
    struct SuspendedEntry {
        expires: Instant,
        reason: HealthEvent,
    }

    /// A `CounterSeries` plus the `spec.period` it was built for, so we
    /// can rebuild on a config edit that changes the window.
    struct SeriesEntry {
        period_secs: u64,
        series: CounterSeries,
    }

    impl SeriesEntry {
        fn new_for(period_secs: u64) -> Self {
            // Aim for ~60 buckets across the window so the threshold
            // check has reasonable temporal resolution without blowing
            // up memory for long windows. For very short windows we
            // fall back to bucket_size = 1s.
            let bucket_size = period_secs.div_ceil(60).max(1);
            let num_buckets =
                period_secs.div_ceil(bucket_size).clamp(1, u8::MAX as u64) as u8;
            Self {
                period_secs,
                series: CounterSeries::with_config(CounterSeriesConfig {
                    num_buckets,
                    bucket_size,
                }),
            }
        }
    }

    /// Counters tracking sliding-window failure series per source, only
    /// populated when at least one `Trigger::Threshold` rule fires events
    /// against that source.
    #[derive(Default)]
    struct Counters {
        unplumbed: Option<SeriesEntry>,
        proxy: Option<SeriesEntry>,
    }

    impl Counters {
        fn series_for(&mut self, event: HealthEvent, period_secs: u64) -> &mut CounterSeries {
            let slot = match event {
                HealthEvent::Unplumbed => &mut self.unplumbed,
                HealthEvent::ProxyUnhealthy => &mut self.proxy,
            };
            match slot {
                Some(entry) if entry.period_secs == period_secs => {}
                _ => *slot = Some(SeriesEntry::new_for(period_secs)),
            }
            &mut slot.as_mut().expect("just populated").series
        }
    }

    /// Live count of entries in `SUSPENDED`. Incremented/decremented
    /// under the same shard write lock that mutates the corresponding
    /// entry, so the count is a faithful mirror of the map size. The
    /// hot-path lookup short-circuits when this is `0`.
    ///
    /// An expired-but-not-yet-pruned entry counts; that's harmless
    /// because `suspension()` checks `expires > now` and returns `None`
    /// for stale entries.
    static SUSPENDED_COUNT: AtomicUsize = AtomicUsize::new(0);

    /// Master state for currently-suspended sources, keyed by name.
    /// The shard write lock provided by `DashMap` is what serializes
    /// `install_suspension` and `prune_expired` against each other
    /// (and against themselves for the same source) and is what keeps
    /// the derived metrics (`SUSPENDED_GAUGE`, `SUSPENSIONS_TOTAL`)
    /// in sync with the map state — metric updates happen inside the
    /// `Entry` / `retain` closures that hold that lock.
    static SUSPENDED: LazyLock<DashMap<String, SuspendedEntry>> = LazyLock::new(DashMap::new);

    /// Cold-path counter state for `Trigger::Threshold` rules.
    static COUNTERS: LazyLock<DashMap<String, Counters>> = LazyLock::new(DashMap::new);

    /// Notified whenever a new entry is inserted into `SUSPENDED`, so
    /// the reaper can re-evaluate its sleep deadline.
    static REAPER_WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);

    /// Ensures the reaper task is spawned exactly once.
    static REAPER_STARTED: Once = Once::new();

    declare_metric! {
        /// Counts connection failures classified as belonging to one of
        /// the source-health failure classes. Increments regardless of
        /// whether `suspend_when_*` is configured on the source, so an
        /// operator can observe the underlying signal before opting in
        /// to auto-suspension.
        ///
        /// {{since('dev')}}
        ///
        /// Labels:
        /// * `source` is the operator-defined egress source name (the
        ///   `name` field of `kumo.make_egress_source`).
        /// * `kind` is one of:
        ///     * `Unplumbed` — the source's local `source_address`
        ///       returned `EADDRNOTAVAIL` on `bind()`. The IP address
        ///       is not currently plumbed on this host.
        ///     * `ProxyUnhealthy` — the configured proxy server was
        ///       unreachable, timed out, or reported a bind failure
        ///       for the requested source address.
        static FAILURES: IntCounterVec(
            "egress_source_connection_failures_total",
            &["source", "kind"],
        );
    }

    declare_metric! {
        /// Increments each time a source transitions into the
        /// auto-suspended state due to one of its `suspend_when_*`
        /// rules firing. Re-triggering the same rule while the
        /// suspension is already active does not increment this
        /// counter; it only counts state transitions.
        ///
        /// {{since('dev')}}
        ///
        /// Labels:
        /// * `source` is the operator-defined egress source name (the
        ///   `name` field of `kumo.make_egress_source`).
        /// * `reason` indicates which rule fired:
        ///     * `Unplumbed` — a `suspend_when_unplumbed` rule fired
        ///       because the source's local `source_address` was not
        ///       plumbed on this host. Plumb the address (or correct
        ///       the configured `source_address`) to resolve.
        ///     * `ProxyUnhealthy` — a `suspend_when_proxy_unhealthy`
        ///       rule fired because the configured proxy server was
        ///       unreachable or rejected the requested source address.
        ///       Investigate the proxy service.
        static SUSPENSIONS_TOTAL: IntCounterVec(
            "egress_source_health_suspensions_total",
            &["source", "reason"],
        );
    }

    declare_metric! {
        /// `1` while an egress source is currently auto-suspended, `0`
        /// otherwise. Pool selection skips a source whose gauge is `1`,
        /// rolling the remaining suspension duration into the per-pool
        /// `min_delay`.
        ///
        /// {{since('dev')}}
        ///
        /// Labels:
        /// * `source` is the operator-defined egress source name (the
        ///   `name` field of `kumo.make_egress_source`).
        /// * `reason` indicates which rule's firing produced the
        ///   current suspension:
        ///     * `Unplumbed` — a `suspend_when_unplumbed` rule fired
        ///       because the source's local `source_address` was not
        ///       plumbed on this host. Plumb the address (or correct
        ///       the configured `source_address`) to resolve.
        ///     * `ProxyUnhealthy` — a `suspend_when_proxy_unhealthy`
        ///       rule fired because the configured proxy server was
        ///       unreachable or rejected the requested source address.
        ///       Investigate the proxy service.
        static SUSPENDED_GAUGE: IntGaugeVec(
            "egress_source_health_suspended",
            &["source", "reason"],
        );
    }

    fn install_suspension(source_name: &str, reason: HealthEvent, duration: Duration) {
        let now = Instant::now();
        let expires = now + duration;

        // Acquire the shard write lock for this source via `entry()`.
        // Each arm computes the appropriate `Transition` variant and
        // delegates all count / metric / log bookkeeping to
        // `apply_transition`, which is the single source of truth for
        // how those derived signals respond to a map change.
        match SUSPENDED.entry(source_name.to_string()) {
            Entry::Vacant(v) => {
                v.insert(SuspendedEntry { expires, reason });
                apply_transition(
                    source_name,
                    Transition::Inserted { reason, duration },
                );
            }
            Entry::Occupied(mut o) => {
                let prior = *o.get();
                let prior_was_active = prior.expires > now;
                *o.get_mut() = SuspendedEntry { expires, reason };
                apply_transition(
                    source_name,
                    Transition::Replaced {
                        prior_reason: prior.reason,
                        prior_was_active,
                        new_reason: reason,
                        duration,
                    },
                );
            }
        }

        ensure_reaper_running();
        REAPER_WAKE.notify_one();
    }

    fn ensure_reaper_running() {
        REAPER_STARTED.call_once(|| {
            tokio::spawn(reaper_loop());
        });
    }

    async fn reaper_loop() {
        loop {
            let next_wakeup = {
                let now = Instant::now();
                let mut earliest: Option<Instant> = None;
                for entry in SUSPENDED.iter() {
                    let exp = entry.expires.max(now);
                    earliest = Some(earliest.map(|e| e.min(exp)).unwrap_or(exp));
                }
                earliest.map(|e| e.saturating_duration_since(now))
            };

            match next_wakeup {
                Some(d) => {
                    tokio::select! {
                        _ = tokio::time::sleep(d) => {}
                        _ = REAPER_WAKE.notified() => {}
                    }
                }
                None => {
                    // No active entries. Park until someone installs one.
                    REAPER_WAKE.notified().await;
                }
            }

            prune_expired();
        }
    }

    fn prune_expired() {
        let now = Instant::now();
        // The retain closure runs under each shard's write lock; like
        // `install_suspension`, all bookkeeping goes through
        // `apply_transition` so the count, gauges, and log line stay
        // in lockstep with the map mutation.
        SUSPENDED.retain(|name, entry| {
            if entry.expires <= now {
                apply_transition(
                    name.as_str(),
                    Transition::Removed { reason: entry.reason },
                );
                false
            } else {
                true
            }
        });
    }

    /// Describes a mutation that just happened to `SUSPENDED`. Every
    /// call to `apply_transition` corresponds to one such mutation;
    /// adding a new mutation kind in the future means adding a variant
    /// here, which forces consideration of all derived bookkeeping in
    /// the same place.
    enum Transition {
        /// A new entry was inserted into a previously vacant slot.
        /// Map size grew by 1; no prior gauge for this source was set.
        Inserted {
            reason: HealthEvent,
            duration: Duration,
        },
        /// An existing entry was replaced in place. Map size unchanged.
        ///
        /// `prior_reason` is the literal reason stored in the slot
        /// before the replace (whether or not it was still active);
        /// `prior_was_active` is whether `prior.expires > now` held at
        /// the moment of the replace. Together they let
        /// `apply_transition` decide whether this is a fresh suspension
        /// transition (counter bumps), an extension (no metric change),
        /// or a reason-change against a stale-expired entry (counter
        /// bumps AND the stale gauge needs explicit clearing).
        Replaced {
            prior_reason: HealthEvent,
            prior_was_active: bool,
            new_reason: HealthEvent,
            duration: Duration,
        },
        /// An expired entry was removed by the reaper. Map size shrank
        /// by 1; the source's gauge for this reason must go to 0.
        Removed { reason: HealthEvent },
    }

    /// Apply the count, gauge, counter, and log effects implied by a
    /// `Transition`. This is the only place in the module that touches
    /// `SUSPENDED_COUNT`, `SUSPENDED_GAUGE`, or `SUSPENSIONS_TOTAL`,
    /// so the derived signals can't drift from the map state in any
    /// future edit unless that edit also passes through this function.
    ///
    /// Must be called while holding the shard write lock for the
    /// entry being mutated, immediately after that mutation. The
    /// `Entry` / `retain` API guarantees this when the call is made
    /// inline from inside the arm or closure.
    fn apply_transition(source_name: &str, t: Transition) {
        match t {
            Transition::Inserted { reason, duration } => {
                SUSPENDED_COUNT.fetch_add(1, Ordering::Relaxed);
                SUSPENDED_GAUGE
                    .with_label_values(&[source_name, reason.label()])
                    .set(1);
                SUSPENSIONS_TOTAL
                    .with_label_values(&[source_name, reason.label()])
                    .inc();
                tracing::warn!(
                    source = source_name,
                    reason = reason.label(),
                    duration = ?duration,
                    "egress source auto-suspended"
                );
            }
            Transition::Replaced {
                prior_reason,
                prior_was_active,
                new_reason,
                duration,
            } => {
                // A pure extension is the only case that produces no
                // metric change: same active suspension, same reason,
                // just a later expiry. Everything else is a fresh
                // transition that bumps the counter.
                if prior_was_active && prior_reason == new_reason {
                    tracing::debug!(
                        source = source_name,
                        reason = new_reason.label(),
                        duration = ?duration,
                        "egress source suspension extended"
                    );
                    return;
                }
                // If the prior entry had a different reason, the gauge
                // for that reason is still 1 (either because the prior
                // suspension is still active under a different reason,
                // or because the reaper hasn't gotten around to
                // clearing the stale-expired entry's gauge). Either
                // way, clear it explicitly.
                if prior_reason != new_reason {
                    SUSPENDED_GAUGE
                        .with_label_values(&[source_name, prior_reason.label()])
                        .set(0);
                }
                SUSPENDED_GAUGE
                    .with_label_values(&[source_name, new_reason.label()])
                    .set(1);
                SUSPENSIONS_TOTAL
                    .with_label_values(&[source_name, new_reason.label()])
                    .inc();
                tracing::warn!(
                    source = source_name,
                    reason = new_reason.label(),
                    duration = ?duration,
                    "egress source auto-suspended"
                );
            }
            Transition::Removed { reason } => {
                SUSPENDED_COUNT.fetch_sub(1, Ordering::Relaxed);
                SUSPENDED_GAUGE
                    .with_label_values(&[source_name, reason.label()])
                    .set(0);
                tracing::info!(
                    source = source_name,
                    reason = reason.label(),
                    "egress source auto-suspension cleared"
                );
            }
        }
    }
}
