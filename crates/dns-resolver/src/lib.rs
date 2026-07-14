use anyhow::Context;
use arc_swap::ArcSwap;
use hickory_resolver::proto::op::ResponseCode;
pub use hickory_resolver::proto::rr::rdata::tlsa::{CertUsage, Matching, Selector, TLSA};
pub use hickory_resolver::proto::rr::Name;
use hickory_resolver::proto::rr::{RData, RecordType};
use hickory_resolver::proto::ProtoError;
use kumo_address::host::HostAddress;
use kumo_address::host_or_socket::HostOrSocketAddress;
use kumo_address::resolvable::ResolvableSocketAddr;
use kumo_log_types::ResolvedAddress;
use kumo_prometheus::declare_metric;
use lruttl::declare_cache;
use rand::prelude::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::timeout;

mod resolver;
#[cfg(feature = "unbound")]
pub use resolver::UnboundResolver;
pub use resolver::{
    ptr_host, reverse_ip, AggregateResolver, Answer, DnsError, HickoryResolver, IpDisplay,
    Resolver, TestResolver,
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
/// Caches domain name to ipv4 records and their DNSSEC secure status
static IPV4_CACHE: LruCacheWithTtl<Name, Arc<IpAddresses>>::new("dns_resolver_ipv4", 1024);
}
declare_cache! {
/// Caches domain name to ipv6 records and their DNSSEC secure status
static IPV6_CACHE: LruCacheWithTtl<Name, Arc<IpAddresses>>::new("dns_resolver_ipv6", 1024);
}
declare_cache! {
/// Caches domain name to the combined set of ipv4 and ipv6 records and their
/// DNSSEC secure status
static IP_CACHE: LruCacheWithTtl<(Name, IpLookupStrategy), Arc<IpAddresses>>::new("dns_resolver_ip", 1024);
}

/// Maximum number of concurrent mx resolves permitted
static MX_MAX_CONCURRENCY: AtomicUsize = AtomicUsize::new(128);
static MX_CONCURRENCY_SEMA: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MX_MAX_CONCURRENCY.load(Ordering::SeqCst)));

/// 5 seconds in ms
static MX_TIMEOUT_MS: AtomicUsize = AtomicUsize::new(5000);

/// 5 minutes in ms
static MX_NEGATIVE_TTL: AtomicUsize = AtomicUsize::new(300 * 1000);

declare_metric! {
/// number of `MailExchanger::resolve` calls currently in progress.
static MX_IN_PROGRESS: IntGauge("dns_mx_resolve_in_progress");
}

declare_metric! {
/// Total number of successful `MailExchanger::resolve` calls
static MX_SUCCESS: IntCounter(
        "dns_mx_resolve_status_ok");
}

declare_metric! {
/// Total number of failed `MailExchanger::resolve` calls.
///
/// Spikes may indicate an issue with your DNS configuration
/// or infrastructure, or may simply indicate that the traffic
/// is destined for bogus addresses.
static MX_FAIL: IntCounter("dns_mx_resolve_status_fail");
}

declare_metric! {
/// Total number of MailExchanger::resolve calls satisfied by level 1 cache.
///
/// Redundant with the newer [lruttl_hit_count{cache_name="dns_resolver_mx"}](lruttl_hit_count.md)
/// metric.
static MX_CACHED: IntCounter("dns_mx_resolve_cache_hit");
}

declare_metric! {
/// Total number of MailExchanger::resolve calls that resulted in an MX DNS request to the next level of cache
///
/// Redundant with the newer [lruttl_miss_count{cache_name="dns_resolver_mx"}](lruttl_miss_count.md)
/// metric.
static MX_QUERIES: IntCounter("dns_mx_resolve_cache_miss");
}

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

/// The outcome of attempting to resolve DANE TLSA records for an MX host,
/// per <https://datatracker.ietf.org/doc/html/rfc7672>.
#[derive(Debug, Clone, PartialEq)]
pub enum DaneStatus {
    /// The host is not DANE-eligible: either the MX host's address (A/AAAA)
    /// RRset was not securely (DNSSEC) resolved, or there was a secure proof
    /// that no TLSA records exist (NODATA/NXDOMAIN). Delivery should continue
    /// using the normal (opportunistic / configured) TLS policy.
    NotApplicable,
    /// Secure, usable DANE-TA(2)/DANE-EE(3) TLSA records were found. The
    /// connection must use TLS and authenticate the peer against these records.
    Records(Vec<TLSA>),
    /// Secure TLSA records were published, but none are usable for DANE SMTP
    /// (e.g. only PKIX-TA(0)/PKIX-EE(1), private/unassigned usages, or
    /// unsupported selector/matching types). Per RFC 7672 section 4.1 the
    /// client must still require STARTTLS but cannot authenticate the peer.
    Unusable,
    /// The secure status of the TLSA records (or of the MX host's address
    /// records) could not be determined, e.g. due to SERVFAIL, a timeout, or a
    /// DNSSEC validation failure (bogus). To preserve downgrade resistance the
    /// delivery must be deferred rather than continuing without authentication.
    TempFail(String),
}

/// Decides whether a single TLSA record is usable for DANE SMTP.
///
/// RFC 7672 section 3.1.3 restricts SMTP DANE to the DANE-TA(2) and DANE-EE(3)
/// certificate usages; PKIX-TA(0), PKIX-EE(1), and private/unassigned usages
/// MUST be treated as unusable. We also require a selector and matching type
/// that we (and OpenSSL) understand, and a digest of the correct length.
fn tlsa_is_usable(tlsa: &TLSA) -> bool {
    match tlsa.cert_usage {
        CertUsage::DaneTa | CertUsage::DaneEe => {}
        _ => return false,
    }
    match tlsa.selector {
        Selector::Full | Selector::Spki => {}
        _ => return false,
    }
    let expected_len = match tlsa.matching {
        Matching::Raw => None,
        Matching::Sha256 => Some(32),
        Matching::Sha512 => Some(64),
        _ => return false,
    };
    match expected_len {
        Some(len) if tlsa.cert_data.len() != len => false,
        _ => true,
    }
}

/// Resolves DANE TLSA records for the given MX host and port, applying the
/// RFC 7672 rules required for downgrade-resistant DANE SMTP.
///
/// `mx_host` must be the MX hostname (RFC 7672 section 3.2.2: the MX hostname is
/// the only reference identifier), not the envelope/routing domain.
///
/// The caller is responsible for only invoking this when the chain to the MX
/// host was securely (DNSSEC) resolved: both the MX RRset (see
/// [`MailExchanger::is_secure`]) and the MX host's address records (see
/// [`ResolvedAddress::is_secure`](kumo_log_types::ResolvedAddress)) must be
/// secure. RFC 7672 section 2.2 requires the address records to be secure
/// before it is safe to rely on TLSA records; we satisfy that by gating on the
/// secure status of the very address we are about to connect to.
pub async fn resolve_dane(mx_host: &str, port: u16) -> anyhow::Result<DaneStatus> {
    let name = fully_qualify(&format!("_{port}._tcp.{mx_host}"))?;
    let answer = RESOLVER.load().resolve(name, RecordType::TLSA).await;
    Ok(classify_tlsa_answer(mx_host, port, answer))
}

/// Maps the result of the TLSA lookup onto a [`DaneStatus`], applying the
/// RFC 7672 rules. Split out from [`resolve_dane`] so that it can be unit
/// tested without a live (DNSSEC validating) resolver.
fn classify_tlsa_answer(mx_host: &str, port: u16, answer: Result<Answer, DnsError>) -> DaneStatus {
    let answer = match answer {
        Ok(answer) => answer,
        Err(err) => {
            // No answer at all (a timeout or other communication/resource
            // failure); the TLSA status is unknown, so we must not downgrade.
            // Failure RCODEs such as SERVFAIL are handled in the match below.
            return DaneStatus::TempFail(format!("TLSA lookup for {mx_host}:{port} failed: {err}"));
        }
    };
    tracing::debug!("resolve_dane {mx_host}:{port} TLSA answer is: {answer:?}");

    if answer.bogus {
        // Bogus records are either tampered with, or due to misconfiguration
        // of the local resolver.
        return DaneStatus::TempFail(format!(
            "TLSA records for {mx_host}:{port} are bogus: {}",
            answer
                .why_bogus
                .as_deref()
                .unwrap_or("DNSSEC validation failed")
        ));
    }

    match answer.response_code {
        ResponseCode::NoError => {}
        // A secure (or insecure) denial of existence means there are no TLSA
        // records; the host is simply not a DANE host.
        ResponseCode::NXDomain => return DaneStatus::NotApplicable,
        // SERVFAIL, REFUSED, NOTIMP, etc. leave the status unknown.
        rcode => {
            return DaneStatus::TempFail(format!(
                "TLSA lookup for {mx_host}:{port} returned {rcode}"
            ));
        }
    }

    // We can only trust TLSA records that were DNSSEC validated.
    if !answer.secure {
        return DaneStatus::NotApplicable;
    }

    let mut published = vec![];
    for r in &answer.records {
        if let RData::TLSA(tlsa) = r {
            published.push(tlsa.clone());
        }
    }

    if published.is_empty() {
        // Secure proof that no TLSA records exist (NODATA).
        return DaneStatus::NotApplicable;
    }

    let mut usable: Vec<TLSA> = published.into_iter().filter(tlsa_is_usable).collect();

    if usable.is_empty() {
        // TLSA records exist but none are usable for DANE SMTP.
        return DaneStatus::Unusable;
    }

    // DNS results are unordered; sort for stable behavior and tests. The TLSA
    // type is an upstream type and does not implement Ord, so sort on its
    // component fields directly.
    usable.sort_by_key(|a| {
        (
            u8::from(a.cert_usage),
            u8::from(a.selector),
            u8::from(a.matching),
            a.cert_data.clone(),
        )
    });

    tracing::info!("resolve_dane {mx_host}:{port} usable TLSA records: {usable:?}");

    DaneStatus::Records(usable)
}

/// The outcome of an explicit `CNAME` lookup used to decide DANE eligibility
/// for an MX host whose address chain was not fully DNSSEC-secure.
///
/// RFC 7672 section 2.2.2 treats an MX host that is a securely published CNAME
/// alias as DANE-eligible at the original (unexpanded) name, even when the
/// alias target lands in an insecure (unsigned) zone: it is the securely
/// published TLSA RRset, not the address records, that authenticates the peer.
#[derive(Debug, PartialEq, Eq)]
pub enum SecureCnameStatus {
    /// The name is a CNAME alias whose alias record was DNSSEC validated, so the
    /// host remains DANE-eligible at its original name.
    SecureAlias,
    /// The name is not a securely published CNAME alias (it is not an alias at
    /// all, or the alias was not DNSSEC validated); it is not the secure-CNAME
    /// case and DANE must not be engaged off the back of it.
    NotSecureAlias,
    /// The secure status of the alias could not be determined (SERVFAIL,
    /// timeout, or a DNSSEC validation failure). To preserve downgrade
    /// resistance the caller must defer rather than continue.
    TempFail(String),
}

/// Performs an explicit `CNAME` lookup for `mx_host` to determine whether it is
/// a securely published alias.
///
/// This is a narrow fallback in the DANE path: when the MX host's address
/// (A/AAAA) records did not resolve securely we may still be looking at a secure
/// CNAME whose target merely lives in an unsigned zone. Querying the `CNAME`
/// type explicitly isolates the alias's own DNSSEC status (a CNAME-type query is
/// answered by the alias RRset and is not chased into the insecure target), and
/// works uniformly across resolver backends because it relies only on the
/// per-answer secure bit rather than per-record validation proofs.
pub async fn resolve_secure_cname(mx_host: &str) -> anyhow::Result<SecureCnameStatus> {
    let name = fully_qualify(mx_host)?;
    let answer = RESOLVER.load().resolve(name, RecordType::CNAME).await;
    Ok(classify_cname_answer(mx_host, answer))
}

/// Maps the result of the explicit `CNAME` lookup onto a [`SecureCnameStatus`].
/// Split out from [`resolve_secure_cname`] so it can be unit tested without a
/// live (DNSSEC validating) resolver.
fn classify_cname_answer(mx_host: &str, answer: Result<Answer, DnsError>) -> SecureCnameStatus {
    let answer = match answer {
        Ok(answer) => answer,
        Err(err) => {
            return SecureCnameStatus::TempFail(format!(
                "CNAME lookup for {mx_host} failed: {err}"
            ));
        }
    };

    if answer.bogus {
        return SecureCnameStatus::TempFail(format!(
            "CNAME records for {mx_host} are bogus: {}",
            answer
                .why_bogus
                .as_deref()
                .unwrap_or("DNSSEC validation failed")
        ));
    }

    match answer.response_code {
        ResponseCode::NoError => {}
        // A denial of existence means there is no CNAME (and possibly no such
        // name); either way it is not a secure alias.
        ResponseCode::NXDomain => return SecureCnameStatus::NotSecureAlias,
        rcode => {
            return SecureCnameStatus::TempFail(format!(
                "CNAME lookup for {mx_host} returned {rcode}"
            ));
        }
    }

    // We can only rely on a CNAME that was DNSSEC validated.
    if !answer.secure {
        return SecureCnameStatus::NotSecureAlias;
    }

    // A secure NODATA answer (no CNAME record) means the host is not an alias.
    if answer.records.iter().any(|r| matches!(r, RData::CNAME(_))) {
        SecureCnameStatus::SecureAlias
    } else {
        SecureCnameStatus::NotSecureAlias
    }
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
    strategy: IpLookupStrategy,
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
                        is_secure: false,
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
                    is_secure: false,
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
                is_secure: false,
            }]);
        }
    }

    match ip_lookup(domain_name, resolver, strategy).await {
        Ok((result, _expires)) => {
            let addrs = result
                .addrs
                .iter()
                .map(|&addr| ResolvedAddress {
                    name: domain_name.to_string(),
                    addr: addr.into(),
                    is_secure: result.secure,
                })
                .collect();
            Ok(addrs)
        }
        Err(err) => anyhow::bail!("{err:#}"),
    }
}

/// Resolve a [`ResolvableSocketAddr`] to a list of concrete [`ResolvedAddress`]es.
///
/// For `UnixDomain`, `V4` and `V6` arms, no DNS lookup is performed; a single
/// `ResolvedAddress` is returned whose `addr` carries the supplied port (for
/// the IP arms). For the `Hostname` arm, an A/AAAA lookup is performed via
/// [`ip_lookup`] and each returned IP is paired with the supplied port.
pub async fn resolve_socket_addr(
    addr: &ResolvableSocketAddr,
    resolver: Option<&dyn Resolver>,
    strategy: IpLookupStrategy,
) -> anyhow::Result<Vec<ResolvedAddress>> {
    match addr {
        ResolvableSocketAddr::UnixDomain(unix) => {
            let name = match unix.as_pathname() {
                Some(path) => path.display().to_string(),
                None => "<unbound unix domain>".to_string(),
            };
            Ok(vec![ResolvedAddress {
                name,
                addr: HostOrSocketAddress::UnixDomain(unix.clone()),
                is_secure: false,
            }])
        }
        ResolvableSocketAddr::V4(sa) => Ok(vec![ResolvedAddress {
            name: sa.to_string(),
            addr: HostOrSocketAddress::V4Socket(sa.clone()),
            is_secure: false,
        }]),
        ResolvableSocketAddr::V6(sa) => Ok(vec![ResolvedAddress {
            name: sa.to_string(),
            addr: HostOrSocketAddress::V6Socket(sa.clone()),
            is_secure: false,
        }]),
        ResolvableSocketAddr::Hostname { host, port } => {
            match ip_lookup(host, resolver, strategy).await {
                Ok((result, _expires)) => Ok(result
                    .addrs
                    .iter()
                    .map(|&ip| {
                        let sa = SocketAddr::new(ip, *port);
                        ResolvedAddress {
                            name: host.clone(),
                            addr: sa.into(),
                            is_secure: result.secure,
                        }
                    })
                    .collect()),
                Err(err) => anyhow::bail!("{err:#}"),
            }
        }
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
    pub async fn resolve_addresses(&self, strategy: IpLookupStrategy) -> ResolvedMxAddresses {
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
                        is_secure: false,
                    });
                    continue;
                }

                match ip_lookup(mx_host, None, strategy).await {
                    Err(err) => {
                        tracing::error!("failed to resolve {mx_host}: {err:#}");
                        continue;
                    }
                    Ok((result, _expires)) => {
                        for addr in result.addrs.iter() {
                            let mut addr: HostOrSocketAddress = (*addr).into();
                            if let Some(port) = opt_port {
                                addr.set_port(port);
                            }
                            by_pref.push(ResolvedAddress {
                                name: mx_host.to_string(),
                                addr,
                                is_secure: result.secure,
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
    /// The list of addresses to which to connect, expressed
    /// in LIFO order
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

        // No MX records: the domain's own A/AAAA records act as the implicit
        // MX. This implicit MX is secure exactly when the MX NODATA response
        // was securely (DNSSEC) resolved, which is common for signed domains
        // that publish no MX (e.g. many `.br` domains).
        return Ok((
            vec![ByPreference {
                hosts: vec![domain_name.to_lowercase().to_ascii()],
                pref: 1,
                is_secure: mx_lookup.secure,
                is_mx: false,
            }],
            mx_lookup.expires,
        ));
    }

    let mut records: Vec<ByPreference> = Vec::with_capacity(mx_records.len());

    for mx_record in mx_records {
        if let RData::MX(mx) = mx_record {
            let pref = mx.preference;
            let host = mx.exchange.to_lowercase().to_string();

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[repr(u8)]
pub enum IpLookupStrategy {
    /// Only query for A (Ipv4) records
    Ipv4Only,
    /// Only query for AAAA (Ipv6) records
    Ipv6Only,
    /// Query for A and AAAA in parallel
    #[default]
    Ipv4AndIpv6,
    /// Query for Ipv6 if that fails, query for Ipv4
    Ipv6ThenIpv4,
    /// Query for Ipv4 if that fails, query for Ipv6 (default)
    Ipv4ThenIpv6,
}

/// A set of resolved IP addresses together with whether the DNS lookup that
/// produced them was DNSSEC validated (secure). The secure flag is what lets
/// the delivery path decide DANE eligibility without performing a second
/// lookup; see [`ResolvedAddress::is_secure`](kumo_log_types::ResolvedAddress).
#[derive(Clone, Debug)]
pub struct IpAddresses {
    pub addrs: Vec<IpAddr>,
    /// True only when every address here came from a DNSSEC-validated lookup.
    pub secure: bool,
}

pub async fn ip_lookup(
    key: &str,
    resolver: Option<&dyn Resolver>,
    strategy: IpLookupStrategy,
) -> anyhow::Result<(Arc<IpAddresses>, Instant)> {
    let key_fq = fully_qualify(key)?;

    if resolver.is_none() {
        if let Some(lookup) = IP_CACHE.lookup(&(key_fq.clone(), strategy)) {
            return Ok((lookup.item, lookup.expiration.into()));
        }
    }

    let (v4, v6) = match strategy {
        IpLookupStrategy::Ipv4AndIpv6 => {
            let (v4, v6) = tokio::join!(ipv4_lookup(key, resolver), ipv6_lookup(key, resolver));
            (Some(v4), Some(v6))
        }
        IpLookupStrategy::Ipv4Only => (Some(ipv4_lookup(key, resolver).await), None),
        IpLookupStrategy::Ipv6Only => (None, Some(ipv6_lookup(key, resolver).await)),
        IpLookupStrategy::Ipv6ThenIpv4 => {
            let v6 = ipv6_lookup(key, resolver).await;
            match v6 {
                Ok((answer, exp)) if answer.addrs.is_empty() => (
                    Some(ipv4_lookup(key, resolver).await),
                    Some(Ok((answer, exp))),
                ),
                Err(err) => (Some(ipv4_lookup(key, resolver).await), Some(Err(err))),
                Ok(res) => (None, Some(Ok(res))),
            }
        }
        IpLookupStrategy::Ipv4ThenIpv6 => {
            let v4 = ipv4_lookup(key, resolver).await;
            match v4 {
                Ok((answer, exp)) if answer.addrs.is_empty() => (
                    Some(Ok((answer, exp))),
                    Some(ipv6_lookup(key, resolver).await),
                ),
                Err(err) => (Some(Err(err)), Some(ipv6_lookup(key, resolver).await)),
                Ok(res) => (Some(Ok(res)), None),
            }
        }
    };

    let mut addrs = vec![];
    let mut any_addr = false;
    let mut all_secure = true;
    let mut errors = vec![];
    let mut expires: Option<Instant> = None;

    for family in [v4, v6] {
        match family {
            Some(Ok((answer, exp))) => {
                expires = Some(match expires {
                    Some(existing) => exp.min(existing),
                    None => exp,
                });
                if !answer.addrs.is_empty() {
                    any_addr = true;
                    all_secure &= answer.secure;
                    addrs.extend_from_slice(&answer.addrs);
                }
            }
            Some(Err(err)) => errors.push(err),
            None => {}
        }
    }

    if addrs.is_empty() && !errors.is_empty() {
        return Err(errors.remove(0));
    }

    let result = Arc::new(IpAddresses {
        addrs,
        // Only claim secure if we actually have addresses and every family that
        // contributed one was DNSSEC validated.
        secure: any_addr && all_secure,
    });
    let exp = expires.unwrap_or_else(Instant::now);

    if resolver.is_none() {
        IP_CACHE
            .insert((key_fq, strategy), result.clone(), exp.into())
            .await;
    }
    Ok((result, exp))
}

pub async fn ipv4_lookup(
    key: &str,
    resolver: Option<&dyn Resolver>,
) -> anyhow::Result<(Arc<IpAddresses>, Instant)> {
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
    let result = Arc::new(IpAddresses {
        addrs: answer.as_addr(),
        secure: answer.secure,
    });
    let expires = answer.expires;
    if resolver.is_none() {
        IPV4_CACHE
            .insert(key_fq, result.clone(), expires.into())
            .await;
    }
    Ok((result, expires))
}

pub async fn ipv6_lookup(
    key: &str,
    resolver: Option<&dyn Resolver>,
) -> anyhow::Result<(Arc<IpAddresses>, Instant)> {
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
    let result = Arc::new(IpAddresses {
        addrs: answer.as_addr(),
        secure: answer.secure,
    });
    let expires = answer.expires;
    if resolver.is_none() {
        IPV6_CACHE
            .insert(key_fq, result.clone(), expires.into())
            .await;
    }
    Ok((result, expires))
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

    fn answer(records: Vec<RData>, secure: bool, response_code: ResponseCode) -> Answer {
        Answer {
            canon_name: None,
            records,
            nxdomain: response_code == ResponseCode::NXDomain,
            secure,
            bogus: false,
            why_bogus: None,
            expires: Instant::now(),
            response_code,
        }
    }

    fn sha256_tlsa(usage: CertUsage, selector: Selector) -> TLSA {
        TLSA::new(usage, selector, Matching::Sha256, vec![0u8; 32])
    }

    fn dane_ee_record() -> TLSA {
        sha256_tlsa(CertUsage::DaneEe, Selector::Spki)
    }

    fn cname_record() -> RData {
        use hickory_resolver::proto::rr::rdata::CNAME;
        RData::CNAME(CNAME(Name::from_ascii("target.unsigned.example.").unwrap()))
    }

    #[test]
    fn cname_answer_classification() {
        // Secure CNAME alias: DANE-eligible at the original name.
        k9::assert_equal!(
            classify_cname_answer(
                "mx.example.com",
                Ok(answer(vec![cname_record()], true, ResponseCode::NoError))
            ),
            SecureCnameStatus::SecureAlias
        );
        // Secure NODATA (not an alias): not the secure-CNAME case.
        k9::assert_equal!(
            classify_cname_answer(
                "mx.example.com",
                Ok(answer(vec![], true, ResponseCode::NoError))
            ),
            SecureCnameStatus::NotSecureAlias
        );
        // A CNAME that was not DNSSEC validated cannot be trusted.
        k9::assert_equal!(
            classify_cname_answer(
                "mx.example.com",
                Ok(answer(vec![cname_record()], false, ResponseCode::NoError))
            ),
            SecureCnameStatus::NotSecureAlias
        );
        // NXDOMAIN: not an alias.
        k9::assert_equal!(
            classify_cname_answer(
                "mx.example.com",
                Ok(answer(vec![], true, ResponseCode::NXDomain))
            ),
            SecureCnameStatus::NotSecureAlias
        );
        // SERVFAIL leaves the status unknown: defer, do not downgrade.
        assert!(matches!(
            classify_cname_answer(
                "mx.example.com",
                Ok(answer(vec![], false, ResponseCode::ServFail))
            ),
            SecureCnameStatus::TempFail(_)
        ));
        // A resolver error is also a temporary failure.
        assert!(matches!(
            classify_cname_answer(
                "mx.example.com",
                Err(DnsError::ResolveFailed("boom".to_string()))
            ),
            SecureCnameStatus::TempFail(_)
        ));
    }

    #[test]
    fn tlsa_usability() {
        // DANE-TA(2) and DANE-EE(3) are usable for DANE SMTP.
        assert!(tlsa_is_usable(&sha256_tlsa(
            CertUsage::DaneTa,
            Selector::Full
        )));
        assert!(tlsa_is_usable(&sha256_tlsa(
            CertUsage::DaneEe,
            Selector::Spki
        )));
        // PKIX-TA(0)/PKIX-EE(1) are not in scope for DANE SMTP (RFC 7672 3.1.3).
        assert!(!tlsa_is_usable(&sha256_tlsa(
            CertUsage::PkixTa,
            Selector::Full
        )));
        assert!(!tlsa_is_usable(&sha256_tlsa(
            CertUsage::PkixEe,
            Selector::Spki
        )));
        // Private and unassigned usages are unusable.
        assert!(!tlsa_is_usable(&sha256_tlsa(
            CertUsage::Private,
            Selector::Spki
        )));
        assert!(!tlsa_is_usable(&sha256_tlsa(
            CertUsage::Unassigned(200),
            Selector::Spki
        )));
        // Unknown selector/matching types and bad digest lengths are unusable.
        assert!(!tlsa_is_usable(&sha256_tlsa(
            CertUsage::DaneEe,
            Selector::Unassigned(7)
        )));
        assert!(!tlsa_is_usable(&TLSA::new(
            CertUsage::DaneEe,
            Selector::Spki,
            Matching::Unassigned(9),
            vec![0u8; 32]
        )));
        assert!(!tlsa_is_usable(&TLSA::new(
            CertUsage::DaneEe,
            Selector::Spki,
            Matching::Sha256,
            vec![0u8; 31]
        )));
        // Raw (full value, matching type 0) has no fixed length.
        assert!(tlsa_is_usable(&TLSA::new(
            CertUsage::DaneEe,
            Selector::Full,
            Matching::Raw,
            vec![0u8; 100]
        )));
    }

    #[test]
    fn tlsa_answer_classification() {
        // Usable, secure DANE-EE record.
        k9::assert_equal!(
            classify_tlsa_answer(
                "mx.example.com",
                25,
                Ok(answer(
                    vec![RData::TLSA(dane_ee_record())],
                    true,
                    ResponseCode::NoError
                ))
            ),
            DaneStatus::Records(vec![dane_ee_record()])
        );
        // Secure NODATA: no TLSA records, not a DANE host.
        k9::assert_equal!(
            classify_tlsa_answer(
                "mx.example.com",
                25,
                Ok(answer(vec![], true, ResponseCode::NoError))
            ),
            DaneStatus::NotApplicable
        );
        // Secure NXDOMAIN: not a DANE host.
        k9::assert_equal!(
            classify_tlsa_answer(
                "mx.example.com",
                25,
                Ok(answer(vec![], true, ResponseCode::NXDomain))
            ),
            DaneStatus::NotApplicable
        );
        // TLSA records present but unvalidated must not be trusted.
        k9::assert_equal!(
            classify_tlsa_answer(
                "mx.example.com",
                25,
                Ok(answer(
                    vec![RData::TLSA(dane_ee_record())],
                    false,
                    ResponseCode::NoError
                ))
            ),
            DaneStatus::NotApplicable
        );
        // Secure records present, but only disallowed usages: unusable, so
        // STARTTLS is required without authentication.
        k9::assert_equal!(
            classify_tlsa_answer(
                "mx.example.com",
                25,
                Ok(answer(
                    vec![RData::TLSA(sha256_tlsa(CertUsage::PkixEe, Selector::Spki))],
                    true,
                    ResponseCode::NoError
                ))
            ),
            DaneStatus::Unusable
        );
        // SERVFAIL leaves the status unknown: defer, do not downgrade.
        assert!(matches!(
            classify_tlsa_answer(
                "mx.example.com",
                25,
                Ok(answer(vec![], false, ResponseCode::ServFail))
            ),
            DaneStatus::TempFail(_)
        ));
        // A resolver error is also a temporary failure.
        assert!(matches!(
            classify_tlsa_answer(
                "mx.example.com",
                25,
                Err(DnsError::ResolveFailed("boom".to_string()))
            ),
            DaneStatus::TempFail(_)
        ));
    }

    /// Confirms that a DNSSEC-validating hickory resolver, via our adapter,
    /// reports signed records as secure and resolves unsigned records as
    /// insecure (rather than failing). Validation requires a DNSSEC-capable
    /// upstream reachable over TCP, since DNSKEY/RRSIG responses are large.
    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn hickory_dnssec_validation() {
        use hickory_resolver::config::{
            ConnectionConfig, NameServerConfig, ProtocolConfig, ResolverConfig,
        };
        use hickory_resolver::net::runtime::TokioRuntimeProvider;
        use hickory_resolver::TokioResolver;

        let mut udp = ConnectionConfig::new(ProtocolConfig::Udp);
        udp.port = 53;
        let mut tcp = ConnectionConfig::new(ProtocolConfig::Tcp);
        tcp.port = 53;
        let config = ResolverConfig::from_parts(
            None,
            vec![],
            vec![NameServerConfig::new(
                "1.1.1.1".parse().unwrap(),
                true,
                vec![udp, tcp],
            )],
        );
        let mut builder =
            TokioResolver::builder_with_config(config, TokioRuntimeProvider::default());
        builder.options_mut().validate = true;
        let resolver = HickoryResolver::from(builder.build().unwrap());

        let tlsa = resolver
            .resolve(
                fully_qualify("_25._tcp.do.havedane.net").unwrap(),
                RecordType::TLSA,
            )
            .await
            .unwrap();
        assert!(tlsa.secure, "signed TLSA should validate as secure");
        assert!(!tlsa.bogus);
        assert!(!tlsa.records.is_empty());

        let unsigned = resolver
            .resolve(fully_qualify("google.com").unwrap(), RecordType::A)
            .await
            .unwrap();
        assert!(!unsigned.secure, "unsigned zone is not secure");
        assert!(!unsigned.bogus);
        assert!(
            !unsigned.records.is_empty(),
            "unsigned zone still resolves successfully"
        );
    }

    /// A securely denied answer (NODATA/NXDOMAIN in a signed zone) must be
    /// reported as secure so that, for example, a securely proven "no MX" can
    /// engage DANE for the implicit MX. An insecure (unsigned) denial must
    /// remain insecure. Exercises the authority-section proof handling on the
    /// hickory backend.
    #[cfg(feature = "live-dns-tests")]
    #[tokio::test]
    async fn hickory_negative_answer_secure_bit() {
        use hickory_resolver::config::{
            ConnectionConfig, NameServerConfig, ProtocolConfig, ResolverConfig,
        };
        use hickory_resolver::net::runtime::TokioRuntimeProvider;
        use hickory_resolver::TokioResolver;

        let mut udp = ConnectionConfig::new(ProtocolConfig::Udp);
        udp.port = 53;
        let mut tcp = ConnectionConfig::new(ProtocolConfig::Tcp);
        tcp.port = 53;
        let config = ResolverConfig::from_parts(
            None,
            vec![],
            vec![NameServerConfig::new(
                "1.1.1.1".parse().unwrap(),
                true,
                vec![udp, tcp],
            )],
        );
        let mut builder =
            TokioResolver::builder_with_config(config, TokioRuntimeProvider::default());
        builder.options_mut().validate = true;
        let resolver = HickoryResolver::from(builder.build().unwrap());

        // Signed zone with no TLSA at the apex: a securely proven NODATA.
        let secure_nodata = resolver
            .resolve(fully_qualify("cloudflare.com").unwrap(), RecordType::TLSA)
            .await
            .unwrap();
        assert!(secure_nodata.records.is_empty());
        assert!(!secure_nodata.bogus);
        assert!(
            secure_nodata.secure,
            "securely denied NODATA in a signed zone should be secure"
        );

        // Unsigned zone NODATA: must not be reported as secure.
        let insecure_nodata = resolver
            .resolve(fully_qualify("mail.anoebis.be").unwrap(), RecordType::AAAA)
            .await
            .unwrap();
        assert!(insecure_nodata.records.is_empty());
        assert!(!insecure_nodata.bogus);
        assert!(
            !insecure_nodata.secure,
            "NODATA in an unsigned zone must not be secure"
        );
    }

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
            v4_loopback
                .resolve_addresses(IpLookupStrategy::default())
                .await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "127.0.0.1",
            addr: 127.0.0.1,
            is_secure: false,
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
            v6_loopback_non_conforming
                .resolve_addresses(IpLookupStrategy::default())
                .await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "::1",
            addr: ::1,
            is_secure: false,
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
            v6_loopback
                .resolve_addresses(IpLookupStrategy::default())
                .await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "::1",
            addr: ::1,
            is_secure: false,
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
            gmail.resolve_addresses(IpLookupStrategy::default()).await,
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

    // Asserts a DNSSEC-secure result, so it only holds when the default
    // resolver validates, i.e. the unbound backend.
    #[cfg(all(feature = "live-dns-tests", feature = "default-unbound"))]
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

    // Asserts a DNSSEC-secure result, so it only holds when the default
    // resolver validates, i.e. the unbound backend.
    #[cfg(all(feature = "live-dns-tests", feature = "default-unbound"))]
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

    // Requires DNSSEC-validated TLSA records, so it only holds when the default
    // resolver validates, i.e. the unbound backend.
    #[cfg(all(feature = "live-dns-tests", feature = "default-unbound"))]
    #[tokio::test]
    async fn tlsa_have_dane() {
        let DaneStatus::Records(tlsa) = resolve_dane("do.havedane.net", 25).await.unwrap() else {
            panic!("expected usable DANE records");
        };
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
