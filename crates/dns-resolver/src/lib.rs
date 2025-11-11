use anyhow::Context;
use arc_swap::ArcSwap;
pub use hickory_resolver::proto::rr::rdata::tlsa::TLSA;
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::proto::ProtoError;
pub use hickory_resolver::Name;
use kumo_address::host::HostAddress;
use kumo_address::host_or_socket::HostOrSocketAddress;
use kumo_log_types::ResolvedAddress;
use lruttl::declare_cache;
use rand::prelude::SliceRandom;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::timeout;

mod resolver;
#[cfg(feature = "unbound")]
pub use resolver::UnboundResolver;
pub use resolver::{
    ptr_host, reverse_ip, AggregateResolver, DnsError, HickoryResolver, IpDisplay, Resolver,
    TestResolver,
};

// An `ArcSwap` can only hold `Sized` types, so we cannot stuff a `dyn Resolver` directly into it.
// Instead, the documentation recommends adding a level of indirection, so we wrap the `Resolver`
// trait object in a `Box`. In the context of DNS requests, the additional pointer chasing should
// not be a significant performance concern.
static RESOLVER: LazyLock<ArcSwap<Box<dyn Resolver>>> =
    LazyLock::new(|| ArcSwap::from_pointee(Box::new(default_resolver())));

declare_cache! {
/// Caches domain name to computed set of MailExchanger records
static MX_CACHE: LruCacheWithTtl<(Name, Option<u16>), Result<Arc<MailExchanger>, String>>::new("dns_resolver_mx", 64 * 1024);
}
declare_cache! {
/// Caches domain name to ipv4 records
static IPV4_CACHE: LruCacheWithTtl<Name, Arc<Vec<IpAddr>>>::new("dns_resolver_ipv4", 1024);
}
declare_cache! {
/// Caches domain name to ipv6 records
static IPV6_CACHE: LruCacheWithTtl<Name, Arc<Vec<IpAddr>>>::new("dns_resolver_ipv6", 1024);
}
declare_cache! {
/// Caches domain name to the combined set of ipv4 and ipv6 records
static IP_CACHE: LruCacheWithTtl<Name, Arc<Vec<IpAddr>>>::new("dns_resolver_ip", 1024);
}

/// Maximum number of concurrent mx resolves permitted
static MX_MAX_CONCURRENCY: AtomicUsize = AtomicUsize::new(128);
static MX_CONCURRENCY_SEMA: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MX_MAX_CONCURRENCY.load(Ordering::SeqCst)));

/// 5 seconds in ms
static MX_TIMEOUT_MS: AtomicUsize = AtomicUsize::new(5000);

/// 5 minutes in ms
static MX_NEGATIVE_TTL: AtomicUsize = AtomicUsize::new(300 * 1000);

static MX_IN_PROGRESS: LazyLock<prometheus::IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "dns_mx_resolve_in_progress",
        "number of MailExchanger::resolve calls currently in progress"
    )
    .unwrap()
});
static MX_SUCCESS: LazyLock<prometheus::IntCounter> = LazyLock::new(|| {
    prometheus::register_int_counter!(
        "dns_mx_resolve_status_ok",
        "total number of successful MailExchanger::resolve calls"
    )
    .unwrap()
});
static MX_FAIL: LazyLock<prometheus::IntCounter> = LazyLock::new(|| {
    prometheus::register_int_counter!(
        "dns_mx_resolve_status_fail",
        "total number of failed MailExchanger::resolve calls"
    )
    .unwrap()
});
static MX_CACHED: LazyLock<prometheus::IntCounter> = LazyLock::new(|| {
    prometheus::register_int_counter!(
        "dns_mx_resolve_cache_hit",
        "total number of MailExchanger::resolve calls satisfied by level 1 cache"
    )
    .unwrap()
});
static MX_QUERIES: LazyLock<prometheus::IntCounter> = LazyLock::new(|| {
    prometheus::register_int_counter!(
        "dns_mx_resolve_cache_miss",
        "total number of MailExchanger::resolve calls that resulted in an \
        MX DNS request to the next level of cache"
    )
    .unwrap()
});

fn default_resolver() -> impl Resolver {
    #[cfg(feature = "default-unbound")]
    return UnboundResolver::new().unwrap();
    #[cfg(not(feature = "default-unbound"))]
    return HickoryResolver::new().expect("Parsing /etc/resolv.conf failed");
}

pub fn set_mx_concurrency_limit(n: usize) {
    MX_MAX_CONCURRENCY.store(n, Ordering::SeqCst);
}

pub fn set_mx_timeout(duration: Duration) -> anyhow::Result<()> {
    let ms = duration
        .as_millis()
        .try_into()
        .context("set_mx_timeout: duration is too large")?;
    MX_TIMEOUT_MS.store(ms, Ordering::Relaxed);
    Ok(())
}

pub fn get_mx_timeout() -> Duration {
    Duration::from_millis(MX_TIMEOUT_MS.load(Ordering::Relaxed) as u64)
}

pub fn set_mx_negative_cache_ttl(duration: Duration) -> anyhow::Result<()> {
    let ms = duration
        .as_millis()
        .try_into()
        .context("set_mx_negative_cache_ttl: duration is too large")?;
    MX_NEGATIVE_TTL.store(ms, Ordering::Relaxed);
    Ok(())
}

pub fn get_mx_negative_ttl() -> Duration {
    Duration::from_millis(MX_NEGATIVE_TTL.load(Ordering::Relaxed) as u64)
}

#[derive(Clone, Debug, Serialize)]
pub struct MailExchanger {
    pub domain_name: String,
    pub hosts: Vec<String>,
    pub site_name: String,
    pub by_pref: BTreeMap<u16, Vec<String>>,
    pub is_domain_literal: bool,
    /// DNSSEC verified
    pub is_secure: bool,
    pub is_mx: bool,
    #[serde(skip)]
    expires: Option<Instant>,
}

pub fn fully_qualify(domain_name: &str) -> Result<Name, ProtoError> {
    let mut name = Name::from_str_relaxed(domain_name)?.to_lowercase();

    // Treat it as fully qualified
    name.set_fqdn(true);

    Ok(name)
}

pub fn reconfigure_resolver(resolver: impl Resolver) {
    RESOLVER.store(Arc::new(Box::new(resolver)));
}

pub fn get_resolver() -> Arc<Box<dyn Resolver>> {
    RESOLVER.load_full()
}

/// Resolves TLSA records for a destination name and port according to
/// <https://datatracker.ietf.org/doc/html/rfc6698#appendix-B.2>
pub async fn resolve_dane(hostname: &str, port: u16) -> anyhow::Result<Vec<TLSA>> {
    let name = fully_qualify(&format!("_{port}._tcp.{hostname}"))?;
    let answer = RESOLVER.load().resolve(name, RecordType::TLSA).await?;
    tracing::debug!("resolve_dane {hostname}:{port} TLSA answer is: {answer:?}");

    if answer.bogus {
        // Bogus records are either tampered with, or due to misconfiguration
        // of the local resolver
        anyhow::bail!(
            "DANE result for {hostname}:{port} unusable because: {}",
            answer
                .why_bogus
                .as_deref()
                .unwrap_or("DNSSEC validation failed")
        );
    }

    let mut result = vec![];
    // We ignore TLSA records unless they are validated; in other words,
    // we'll return an empty list (without raising an error) if the resolver
    // is not configured to verify DNSSEC
    if answer.secure {
        for r in &answer.records {
            if let Some(tlsa) = r.as_tlsa() {
                result.push(tlsa.clone());
            }
        }
        // DNS results are unordered. For the sake of tests,
        // sort these records.
        // Unfortunately, the TLSA type is nor Ord so we
        // convert to string and order by that, which is a bit
        // wasteful but the cardinality of TLSA records is
        // generally low
        result.sort_by_key(|a| a.to_string());
    }

    tracing::info!("resolve_dane {hostname}:{port} result is: {result:?}");

    Ok(result)
}

/// If the provided parameter ends with `:PORT` and `PORT` is a valid u16,
/// then crack apart and return the LABEL and PORT number portions.
/// Otherwise, returns None
pub fn has_colon_port(a: &str) -> Option<(&str, u16)> {
    let (label, maybe_port) = a.rsplit_once(':')?;

    // v6 addresses can look like `::1` and confuse us. Try not
    // to be confused here
    if label.contains(':') {
        return None;
    }

    let port = maybe_port.parse::<u16>().ok()?;
    Some((label, port))
}

/// Helper to reason about a domain name string.
/// It can either be name that needs to be resolved, or some kind
/// of IP literal.
/// We also allow for an optional port number to be present in
/// the domain name string.
pub enum DomainClassification {
    /// A DNS Name pending resolution, plus an optional port number
    Domain(Name, Option<u16>),
    /// A literal IP address (no port), or socket address (with port)
    Literal(HostOrSocketAddress),
}

impl DomainClassification {
    pub fn classify(domain_name: &str) -> anyhow::Result<Self> {
        let (domain_name, mut opt_port) = match has_colon_port(domain_name) {
            Some((domain_name, port)) => (domain_name, Some(port)),
            None => (domain_name, None),
        };

        if domain_name.starts_with('[') {
            if !domain_name.ends_with(']') {
                anyhow::bail!(
                    "domain_name `{domain_name}` is a malformed literal \
                     domain with no trailing `]`"
                );
            }

            let lowered = domain_name.to_ascii_lowercase();
            let literal = &lowered[1..lowered.len() - 1];

            let literal = match has_colon_port(literal) {
                Some((_, _)) if opt_port.is_some() => {
                    anyhow::bail!("invalid address: `{domain_name}` specifies a port both inside and outside a literal address enclosed in square brackets");
                }
                Some((literal, port)) => {
                    opt_port.replace(port);
                    literal
                }
                None => literal,
            };

            if let Some(v6_literal) = literal.strip_prefix("ipv6:") {
                match v6_literal.parse::<Ipv6Addr>() {
                    Ok(addr) => {
                        let mut host_addr: HostOrSocketAddress = addr.into();
                        if let Some(port) = opt_port {
                            host_addr.set_port(port);
                        }
                        return Ok(Self::Literal(host_addr));
                    }
                    Err(err) => {
                        anyhow::bail!("invalid ipv6 address: `{v6_literal}`: {err:#}");
                    }
                }
            }

            // Try to interpret the literal as either an IPv4 or IPv6 address.
            // Note that RFC5321 doesn't actually permit using an untagged
            // IPv6 address, so this is non-conforming behavior.
            match literal.parse::<IpAddr>() {
                Ok(ip_addr) => {
                    let mut host_addr: HostOrSocketAddress = ip_addr.into();
                    if let Some(port) = opt_port {
                        host_addr.set_port(port);
                    }
                    return Ok(Self::Literal(host_addr));
                }
                Err(err) => {
                    anyhow::bail!("invalid address: `{literal}`: {err:#}");
                }
            }
        }

        let name_fq = fully_qualify(domain_name)?;
        Ok(Self::Domain(name_fq, opt_port))
    }

    pub fn has_port(&self) -> bool {
        match self {
            Self::Domain(_, Some(_)) => true,
            Self::Domain(_, None) => false,
            Self::Literal(addr) => addr.port().is_some(),
        }
    }
}

pub async fn resolve_a_or_aaaa(
    domain_name: &str,
    resolver: Option<&dyn Resolver>,
) -> anyhow::Result<Vec<ResolvedAddress>> {
    if domain_name.starts_with('[') {
        // It's a literal address, no DNS lookup necessary

        if !domain_name.ends_with(']') {
            anyhow::bail!(
                "domain_name `{domain_name}` is a malformed literal \
                     domain with no trailing `]`"
            );
        }

        let lowered = domain_name.to_ascii_lowercase();
        let literal = &lowered[1..lowered.len() - 1];

        if let Some(v6_literal) = literal.strip_prefix("ipv6:") {
            match v6_literal.parse::<Ipv6Addr>() {
                Ok(addr) => {
                    return Ok(vec![ResolvedAddress {
                        name: domain_name.to_string(),
                        addr: std::net::IpAddr::V6(addr).into(),
                    }]);
                }
                Err(err) => {
                    anyhow::bail!("invalid ipv6 address: `{v6_literal}`: {err:#}");
                }
            }
        }

        // Try to interpret the literal as either an IPv4 or IPv6 address.
        // Note that RFC5321 doesn't actually permit using an untagged
        // IPv6 address, so this is non-conforming behavior.
        match literal.parse::<HostAddress>() {
            Ok(addr) => {
                return Ok(vec![ResolvedAddress {
                    name: domain_name.to_string(),
                    addr: addr.into(),
                }]);
            }
            Err(err) => {
                anyhow::bail!("invalid address: `{literal}`: {err:#}");
            }
        }
    } else {
        // Maybe its a unix domain socket path
        if let Ok(addr) = domain_name.parse::<HostAddress>() {
            return Ok(vec![ResolvedAddress {
                name: domain_name.to_string(),
                addr: addr.into(),
            }]);
        }
    }

    match ip_lookup(domain_name, resolver).await {
        Ok((addrs, _expires)) => {
            let addrs = addrs
                .iter()
                .map(|&addr| ResolvedAddress {
                    name: domain_name.to_string(),
                    addr: addr.into(),
                })
                .collect();
            Ok(addrs)
        }
        Err(err) => anyhow::bail!("{err:#}"),
    }
}

impl MailExchanger {
    pub async fn resolve(domain_name: &str) -> anyhow::Result<Arc<Self>> {
        MX_IN_PROGRESS.inc();
        let result = Self::resolve_impl(domain_name).await;
        MX_IN_PROGRESS.dec();
        if result.is_ok() {
            MX_SUCCESS.inc();
        } else {
            MX_FAIL.inc();
        }
        result
    }

    async fn resolve_impl(domain_name: &str) -> anyhow::Result<Arc<Self>> {
        let (name_fq, opt_port) = match DomainClassification::classify(domain_name)? {
            DomainClassification::Literal(addr) => {
                let mut by_pref = BTreeMap::new();
                by_pref.insert(1, vec![addr.to_string()]);
                return Ok(Arc::new(Self {
                    domain_name: domain_name.to_string(),
                    hosts: vec![addr.to_string()],
                    site_name: addr.to_string(),
                    by_pref,
                    is_domain_literal: true,
                    is_secure: false,
                    is_mx: false,
                    expires: None,
                }));
            }
            DomainClassification::Domain(name_fq, opt_port) => (name_fq, opt_port),
        };

        let lookup_result = MX_CACHE
            .get_or_try_insert(
                &(name_fq.clone(), opt_port),
                |mx_result| {
                    if let Ok(mx) = mx_result {
                        if let Some(exp) = mx.expires {
                            return exp
                                .checked_duration_since(std::time::Instant::now())
                                .unwrap_or_else(|| Duration::from_secs(10));
                        }
                    }
                    get_mx_negative_ttl()
                },
                async {
                    MX_QUERIES.inc();
                    let start = Instant::now();
                    let (mut by_pref, expires) = match lookup_mx_record(&name_fq).await {
                        Ok((by_pref, expires)) => (by_pref, expires),
                        Err(err) => {
                            let error = format!(
                                "MX lookup for {domain_name} failed after {elapsed:?}: {err:#}",
                                elapsed = start.elapsed()
                            );
                            return Ok::<Result<Arc<MailExchanger>, String>, anyhow::Error>(Err(
                                error,
                            ));
                        }
                    };

                    let mut hosts = vec![];
                    for pref in &mut by_pref {
                        for host in &mut pref.hosts {
                            if let Some(port) = opt_port {
                                *host = format!("{host}:{port}");
                            };
                            hosts.push(host.to_string());
                        }
                    }

                    let is_secure = by_pref.iter().all(|p| p.is_secure);
                    let is_mx = by_pref.iter().all(|p| p.is_mx);

                    let by_pref = by_pref
                        .into_iter()
                        .map(|pref| (pref.pref, pref.hosts))
                        .collect();

                    let site_name = factor_names(&hosts);
                    let mx = Self {
                        hosts,
                        domain_name: name_fq.to_ascii(),
                        site_name,
                        by_pref,
                        is_domain_literal: false,
                        is_secure,
                        is_mx,
                        expires: Some(expires),
                    };

                    Ok(Ok(Arc::new(mx)))
                },
            )
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;

        if !lookup_result.is_fresh {
            MX_CACHED.inc();
        }

        lookup_result.item.map_err(|err| anyhow::anyhow!("{err}"))
    }

    pub fn has_expired(&self) -> bool {
        match self.expires {
            Some(deadline) => deadline <= Instant::now(),
            None => false,
        }
    }

    /// Returns the list of resolve MX hosts in *reverse* preference
    /// order; the first one to try is the last element.
    /// smtp_dispatcher.rs relies on this ordering, as it will pop
    /// off candidates until it has exhausted its connection plan.
    pub async fn resolve_addresses(&self) -> ResolvedMxAddresses {
        let mut result = vec![];

        for hosts in self.by_pref.values().rev() {
            let mut by_pref = vec![];

            for mx_host in hosts {
                // '.' is a null mx; skip trying to resolve it
                if mx_host == "." {
                    return ResolvedMxAddresses::NullMx;
                }

                // Handle the literal address case
                let (mx_host, opt_port) = match has_colon_port(mx_host) {
                    Some((domain_name, port)) => (domain_name, Some(port)),
                    None => (mx_host.as_str(), None),
                };
                if let Ok(addr) = mx_host.parse::<IpAddr>() {
                    let mut addr: HostOrSocketAddress = addr.into();
                    if let Some(port) = opt_port {
                        addr.set_port(port);
                    }
                    by_pref.push(ResolvedAddress {
                        name: mx_host.to_string(),
                        addr: addr.into(),
                    });
                    continue;
                }

                match ip_lookup(mx_host, None).await {
                    Err(err) => {
                        tracing::error!("failed to resolve {mx_host}: {err:#}");
                        continue;
                    }
                    Ok((addresses, _expires)) => {
                        for addr in addresses.iter() {
                            let mut addr: HostOrSocketAddress = (*addr).into();
                            if let Some(port) = opt_port {
                                addr.set_port(port);
                            }
                            by_pref.push(ResolvedAddress {
                                name: mx_host.to_string(),
                                addr,
                            });
                        }
                    }
                }
            }

            // Randomize the list of addresses within this preference
            // level. This probablistically "load balances" outgoing
            // traffic across MX hosts with equal preference value.
            let mut rng = rand::thread_rng();
            by_pref.shuffle(&mut rng);
            result.append(&mut by_pref);
        }
        ResolvedMxAddresses::Addresses(result)
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum ResolvedMxAddresses {
    NullMx,
    Addresses(Vec<ResolvedAddress>),
}

struct ByPreference {
    hosts: Vec<String>,
    pref: u16,
    is_secure: bool,
    is_mx: bool,
}

async fn lookup_mx_record(domain_name: &Name) -> anyhow::Result<(Vec<ByPreference>, Instant)> {
    let mx_lookup = timeout(get_mx_timeout(), async {
        let _permit = MX_CONCURRENCY_SEMA.acquire().await;
        RESOLVER
            .load()
            .resolve(domain_name.clone(), RecordType::MX)
            .await
    })
    .await??;
    let mx_records = mx_lookup.records;

    if mx_records.is_empty() {
        if mx_lookup.nxdomain {
            anyhow::bail!("NXDOMAIN");
        }

        return Ok((
            vec![ByPreference {
                hosts: vec![domain_name.to_lowercase().to_ascii()],
                pref: 1,
                is_secure: false,
                is_mx: false,
            }],
            mx_lookup.expires,
        ));
    }

    let mut records: Vec<ByPreference> = Vec::with_capacity(mx_records.len());

    for mx_record in mx_records {
        if let Some(mx) = mx_record.as_mx() {
            let pref = mx.preference();
            let host = mx.exchange().to_lowercase().to_string();

            if let Some(record) = records.iter_mut().find(|r| r.pref == pref) {
                record.hosts.push(host);
            } else {
                records.push(ByPreference {
                    hosts: vec![host],
                    pref,
                    is_secure: mx_lookup.secure,
                    is_mx: true,
                });
            }
        }
    }

    // Sort by preference
    records.sort_unstable_by(|a, b| a.pref.cmp(&b.pref));

    // Sort the hosts at each preference level to produce the
    // overall ordered list of hosts for this site
    for mx in &mut records {
        mx.hosts.sort();
    }

    Ok((records, mx_lookup.expires))
}

pub async fn ip_lookup(
    key: &str,
    resolver: Option<&dyn Resolver>,
) -> anyhow::Result<(Arc<Vec<IpAddr>>, Instant)> {
    let key_fq = fully_qualify(key)?;

    if resolver.is_none() {
        if let Some(lookup) = IP_CACHE.lookup(&key_fq) {
            return Ok((lookup.item, lookup.expiration.into()));
        }
    }

    let (v4, v6) = tokio::join!(ipv4_lookup(key, resolver), ipv6_lookup(key, resolver));

    let mut results = vec![];
    let mut errors = vec![];
    let mut expires = None;

    match v4 {
        Ok((addrs, exp)) => {
            expires.replace(exp);
            for a in addrs.iter() {
                results.push(*a);
            }
        }
        Err(err) => errors.push(err),
    }

    match v6 {
        Ok((addrs, exp)) => {
            let exp = match expires.take() {
                Some(existing) => exp.min(existing),
                None => exp,
            };
            expires.replace(exp);

            for a in addrs.iter() {
                results.push(*a);
            }
        }
        Err(err) => errors.push(err),
    }

    if results.is_empty() && !errors.is_empty() {
        return Err(errors.remove(0));
    }

    let addr = Arc::new(results);
    let exp = expires.take().unwrap_or_else(Instant::now);

    if resolver.is_none() {
        IP_CACHE.insert(key_fq, addr.clone(), exp.into()).await;
    }
    Ok((addr, exp))
}

pub async fn ipv4_lookup(
    key: &str,
    resolver: Option<&dyn Resolver>,
) -> anyhow::Result<(Arc<Vec<IpAddr>>, Instant)> {
    let key_fq = fully_qualify(key)?;
    if resolver.is_none() {
        if let Some(lookup) = IPV4_CACHE.lookup(&key_fq) {
            return Ok((lookup.item, lookup.expiration.into()));
        }
    }

    let answer = match resolver {
        Some(r) => r.resolve(key_fq.clone(), RecordType::A).await?,
        None => {
            RESOLVER
                .load()
                .resolve(key_fq.clone(), RecordType::A)
                .await?
        }
    };
    let ips = answer.as_addr();

    let ips = Arc::new(ips);
    let expires = answer.expires;
    if resolver.is_none() {
        IPV4_CACHE.insert(key_fq, ips.clone(), expires.into()).await;
    }
    Ok((ips, expires))
}

pub async fn ipv6_lookup(
    key: &str,
    resolver: Option<&dyn Resolver>,
) -> anyhow::Result<(Arc<Vec<IpAddr>>, Instant)> {
    let key_fq = fully_qualify(key)?;
    if resolver.is_none() {
        if let Some(lookup) = IPV6_CACHE.lookup(&key_fq) {
            return Ok((lookup.item, lookup.expiration.into()));
        }
    }

    let answer = match resolver {
        Some(r) => r.resolve(key_fq.clone(), RecordType::AAAA).await?,
        None => {
            RESOLVER
                .load()
                .resolve(key_fq.clone(), RecordType::AAAA)
                .await?
        }
    };
    let ips = answer.as_addr();

    let ips = Arc::new(ips);
    let expires = answer.expires;
    if resolver.is_none() {
        IPV6_CACHE.insert(key_fq, ips.clone(), expires.into()).await;
    }
    Ok((ips, expires))
}

/// Given a list of host names, produce a pseudo-regex style alternation list
/// of the different elements of the hostnames.
/// The goal is to produce a more compact representation of the name list
/// with the common components factored out.
fn factor_names<S: AsRef<str>>(name_strings: &[S]) -> String {
    let mut max_element_count = 0;

    let mut names = vec![];

    for name in name_strings {
        let (name, opt_port) = match has_colon_port(name.as_ref()) {
            Some((name, port)) => (name, Some(port)),
            None => (name.as_ref(), None),
        };
        if let Ok(name) = fully_qualify(name) {
            names.push((name.to_lowercase(), opt_port));
        }
    }

    let mut elements: Vec<Vec<&str>> = vec![];

    let mut split_names = vec![];
    for (name, opt_port) in names {
        let mut fields: Vec<_> = name
            .iter()
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect();
        if let Some(port) = opt_port {
            fields.last_mut().map(|s| {
                s.push_str(&format!(":{port}"));
            });
        }
        fields.reverse();
        max_element_count = max_element_count.max(fields.len());
        split_names.push(fields);
    }

    fn add_element<'a>(elements: &mut Vec<Vec<&'a str>>, field: &'a str, i: usize) {
        match elements.get_mut(i) {
            Some(ele) => {
                if !ele.contains(&field) {
                    ele.push(field);
                }
            }
            None => {
                elements.push(vec![field]);
            }
        }
    }

    for fields in &split_names {
        for (i, field) in fields.iter().enumerate() {
            add_element(&mut elements, field, i);
        }
        for i in fields.len()..max_element_count {
            add_element(&mut elements, "?", i);
        }
    }

    let mut result = vec![];
    for mut ele in elements {
        let has_q = ele.contains(&"?");
        ele.retain(|&e| e != "?");
        let mut item_text = if ele.len() == 1 {
            ele[0].to_string()
        } else {
            format!("({})", ele.join("|"))
        };
        if has_q {
            item_text.push('?');
        }
        result.push(item_text);
    }
    result.reverse();

    result.join(".")
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn literal_resolve() {
        let v4_loopback = MailExchanger::resolve("[127.0.0.1]").await.unwrap();
        k9::snapshot!(
            &v4_loopback,
            r#"
MailExchanger {
    domain_name: "[127.0.0.1]",
    hosts: [
        "127.0.0.1",
    ],
    site_name: "127.0.0.1",
    by_pref: {
        1: [
            "127.0.0.1",
        ],
    },
    is_domain_literal: true,
    is_secure: false,
    is_mx: false,
    expires: None,
}
"#
        );
        k9::snapshot!(
            v4_loopback.resolve_addresses().await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "127.0.0.1",
            addr: 127.0.0.1,
        },
    ],
)
"#
        );

        let v6_loopback_non_conforming = MailExchanger::resolve("[::1]").await.unwrap();
        k9::snapshot!(
            &v6_loopback_non_conforming,
            r#"
MailExchanger {
    domain_name: "[::1]",
    hosts: [
        "::1",
    ],
    site_name: "::1",
    by_pref: {
        1: [
            "::1",
        ],
    },
    is_domain_literal: true,
    is_secure: false,
    is_mx: false,
    expires: None,
}
"#
        );
        k9::snapshot!(
            v6_loopback_non_conforming.resolve_addresses().await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "::1",
            addr: ::1,
        },
    ],
)
"#
        );

        let v6_loopback = MailExchanger::resolve("[IPv6:::1]").await.unwrap();
        k9::snapshot!(
            &v6_loopback,
            r#"
MailExchanger {
    domain_name: "[IPv6:::1]",
    hosts: [
        "::1",
    ],
    site_name: "::1",
    by_pref: {
        1: [
            "::1",
        ],
    },
    is_domain_literal: true,
    is_secure: false,
    is_mx: false,
    expires: None,
}
"#
        );
        k9::snapshot!(
            v6_loopback.resolve_addresses().await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "::1",
            addr: ::1,
        },
    ],
)
"#
        );
    }

    #[test]
    fn name_factoring() {
        assert_eq!(
            factor_names(&[
                "mta5.am0.yahoodns.net",
                "mta6.am0.yahoodns.net",
                "mta7.am0.yahoodns.net"
            ]),
            "(mta5|mta6|mta7).am0.yahoodns.net".to_string()
        );

        // Verify that the case is normalized to lowercase
        assert_eq!(
            factor_names(&[
                "mta5.AM0.yahoodns.net",
                "mta6.am0.yAHOodns.net",
                "mta7.am0.yahoodns.net"
            ]),
            "(mta5|mta6|mta7).am0.yahoodns.net".to_string()
        );

        // When the names have mismatched lengths, do we produce
        // something reasonable?
        assert_eq!(
            factor_names(&[
                "gmail-smtp-in.l.google.com",
                "alt1.gmail-smtp-in.l.google.com",
                "alt2.gmail-smtp-in.l.google.com",
                "alt3.gmail-smtp-in.l.google.com",
                "alt4.gmail-smtp-in.l.google.com",
            ]),
            "(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com".to_string()
        );

        assert_eq!(
            factor_names(&[
                "mta5.am0.yahoodns.net:123",
                "mta6.am0.yahoodns.net:123",
                "mta7.am0.yahoodns.net:123"
            ]),
            "(mta5|mta6|mta7).am0.yahoodns.net:123".to_string()
        );
        assert_eq!(
            factor_names(&[
                "mta5.am0.yahoodns.net:123",
                "mta6.am0.yahoodns.net:456",
                "mta7.am0.yahoodns.net:123"
            ]),
            "(mta5|mta6|mta7).am0.yahoodns.(net:123|net:456)".to_string()
        );
    }

    /// Verify that the order is preserved and that we treat these two
    /// examples of differently ordered sets of the same names as two
    /// separate site name strings
    #[test]
    fn mx_order_name_factor() {
        assert_eq!(
            factor_names(&[
                "example-com.mail.protection.outlook.com.",
                "mx-biz.mail.am0.yahoodns.net.",
                "mx-biz.mail.am0.yahoodns.net.",
            ]),
            "(example-com|mx-biz).mail.(protection|am0).(outlook|yahoodns).(com|net)".to_string()
        );
        assert_eq!(
            factor_names(&[
                "mx-biz.mail.am0.yahoodns.net.",
                "mx-biz.mail.am0.yahoodns.net.",
                "example-com.mail.protection.outlook.com.",
            ]),
            "(mx-biz|example-com).mail.(am0|protection).(yahoodns|outlook).(net|com)".to_string()
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn lookup_gmail_mx() {
        let mut gmail = (*MailExchanger::resolve("gmail.com").await.unwrap()).clone();
        gmail.expires.take();
        k9::snapshot!(
            &gmail,
            r#"
MailExchanger {
    domain_name: "gmail.com.",
    hosts: [
        "gmail-smtp-in.l.google.com.",
        "alt1.gmail-smtp-in.l.google.com.",
        "alt2.gmail-smtp-in.l.google.com.",
        "alt3.gmail-smtp-in.l.google.com.",
        "alt4.gmail-smtp-in.l.google.com.",
    ],
    site_name: "(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com",
    by_pref: {
        5: [
            "gmail-smtp-in.l.google.com.",
        ],
        10: [
            "alt1.gmail-smtp-in.l.google.com.",
        ],
        20: [
            "alt2.gmail-smtp-in.l.google.com.",
        ],
        30: [
            "alt3.gmail-smtp-in.l.google.com.",
        ],
        40: [
            "alt4.gmail-smtp-in.l.google.com.",
        ],
    },
    is_domain_literal: false,
    is_secure: false,
    is_mx: true,
    expires: None,
}
"#
        );

        // This is a bad thing to have in a snapshot test really,
        // but the whole set of live-dns-tests are already inherently
        // unstable and flakey anyway.
        // The main thing we expect to see here is that the list of
        // names starts with alt4 and goes backwards through the priority
        // order such that the last element is gmail-smtp.
        // We expect the addresses within a given preference level to
        // be randomized, because that is what resolve_addresses does.
        k9::snapshot!(
            gmail.resolve_addresses().await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "alt4.gmail-smtp-in.l.google.com.",
            addr: 2607:f8b0:4023:401::1b,
        },
        ResolvedAddress {
            name: "alt4.gmail-smtp-in.l.google.com.",
            addr: 173.194.77.27,
        },
        ResolvedAddress {
            name: "alt3.gmail-smtp-in.l.google.com.",
            addr: 2607:f8b0:4023:1::1a,
        },
        ResolvedAddress {
            name: "alt3.gmail-smtp-in.l.google.com.",
            addr: 172.253.113.26,
        },
        ResolvedAddress {
            name: "alt2.gmail-smtp-in.l.google.com.",
            addr: 2607:f8b0:4001:c1d::1b,
        },
        ResolvedAddress {
            name: "alt2.gmail-smtp-in.l.google.com.",
            addr: 74.125.126.27,
        },
        ResolvedAddress {
            name: "alt1.gmail-smtp-in.l.google.com.",
            addr: 2607:f8b0:4003:c04::1b,
        },
        ResolvedAddress {
            name: "alt1.gmail-smtp-in.l.google.com.",
            addr: 108.177.104.27,
        },
        ResolvedAddress {
            name: "gmail-smtp-in.l.google.com.",
            addr: 2607:f8b0:4023:c06::1b,
        },
        ResolvedAddress {
            name: "gmail-smtp-in.l.google.com.",
            addr: 142.251.2.26,
        },
    ],
)
"#
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn lookup_punycode_no_mx_only_a() {
        let mx = MailExchanger::resolve("xn--bb-eka.at").await.unwrap();
        assert_eq!(mx.domain_name, "xn--bb-eka.at.");
        assert_eq!(mx.hosts[0], "xn--bb-eka.at.");
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn lookup_bogus_aasland() {
        let err = MailExchanger::resolve("not-mairs.aasland.com")
            .await
            .unwrap_err();
        k9::snapshot!(err, "MX lookup for not-mairs.aasland.com failed: NXDOMAIN");
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn lookup_example_com() {
        // Has a NULL MX record
        let mx = MailExchanger::resolve("example.com").await.unwrap();
        k9::snapshot!(
            mx,
            r#"
MailExchanger {
    domain_name: "example.com.",
    hosts: [
        ".",
    ],
    site_name: "",
    by_pref: {
        0: [
            ".",
        ],
    },
    is_domain_literal: false,
    is_secure: true,
    is_mx: true,
}
"#
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn lookup_have_dane() {
        let mx = MailExchanger::resolve("do.havedane.net").await.unwrap();
        k9::snapshot!(
            mx,
            r#"
MailExchanger {
    domain_name: "do.havedane.net.",
    hosts: [
        "do.havedane.net.",
    ],
    site_name: "do.havedane.net",
    by_pref: {
        10: [
            "do.havedane.net.",
        ],
    },
    is_domain_literal: false,
    is_secure: true,
    is_mx: true,
}
"#
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn tlsa_have_dane() {
        let tlsa = resolve_dane("do.havedane.net", 25).await.unwrap();
        k9::snapshot!(
            tlsa,
            "
[
    TLSA {
        cert_usage: TrustAnchor,
        selector: Spki,
        matching: Sha256,
        cert_data: [
            39,
            182,
            148,
            181,
            29,
            31,
            239,
            136,
            133,
            55,
            42,
            207,
            179,
            145,
            147,
            117,
            151,
            34,
            183,
            54,
            176,
            66,
            104,
            100,
            220,
            28,
            121,
            208,
            101,
            31,
            239,
            115,
        ],
    },
    TLSA {
        cert_usage: DomainIssued,
        selector: Spki,
        matching: Sha256,
        cert_data: [
            85,
            58,
            207,
            136,
            249,
            238,
            24,
            204,
            170,
            230,
            53,
            202,
            84,
            15,
            50,
            203,
            132,
            172,
            167,
            124,
            71,
            145,
            102,
            130,
            188,
            181,
            66,
            213,
            29,
            170,
            135,
            31,
        ],
    },
]
"
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn mx_lookup_www_example_com() {
        // Has no MX, should fall back to A lookup
        let mx = MailExchanger::resolve("www.example.com").await.unwrap();
        k9::snapshot!(
            mx,
            r#"
MailExchanger {
    domain_name: "www.example.com.",
    hosts: [
        "www.example.com.",
    ],
    site_name: "www.example.com",
    by_pref: {
        1: [
            "www.example.com.",
        ],
    },
    is_domain_literal: false,
    is_secure: false,
    is_mx: false,
}
"#
        );
    }

    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn txt_lookup_gmail() {
        let name = Name::from_str_relaxed("_mta-sts.gmail.com").unwrap();
        let answer = get_resolver().resolve(name, RecordType::TXT).await.unwrap();
        k9::snapshot!(
            answer.as_txt(),
            r#"
[
    "v=STSv1; id=20190429T010101;",
]
"#
        );
    }
}
