use crate::site_name::factor_names;
use anyhow::Context;
use dns_resolver::{
    get_resolver, has_colon_port, ip_lookup, DnsError, DomainClassification, IpLookupStrategy,
    Name, Resolver,
};
use hickory_resolver::proto::rr::{RData, RecordType};
use kumo_address::host_or_socket::HostOrSocketAddress;
use kumo_log_types::ResolvedAddress;
use kumo_prometheus::declare_metric;
use lruttl::declare_cache;
use mta_sts::policy::MtaStsPolicy;
pub use mta_sts::policy::PolicyMode;
use rand::prelude::SliceRandom;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::timeout;

mod site_name;

/// Whether MX resolution consults MTA-STS policies. Defaults to true because
/// honoring a destination's published MTA-STS policy is the correct default.
/// Toggled via `kumo.dns.set_mta_sts_enabled`.
static MTA_STS_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_mta_sts_enabled(enabled: bool) {
    MTA_STS_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_mta_sts_enabled() -> bool {
    MTA_STS_ENABLED.load(Ordering::Relaxed)
}

/// When a policy fetch fails transiently we don't want to pin a "no policy"
/// result for the full DNS TTL, so we cap the cached entry to this interval
/// to re-attempt the policy fetch sooner.
const MTA_STS_FETCH_RETRY: Duration = Duration::from_secs(300);

/// Maximum number of concurrent mx resolves permitted
static MX_MAX_CONCURRENCY: AtomicUsize = AtomicUsize::new(128);
static MX_CONCURRENCY_SEMA: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(MX_MAX_CONCURRENCY.load(Ordering::SeqCst)));

/// 5 seconds in ms
static MX_TIMEOUT_MS: AtomicUsize = AtomicUsize::new(5000);

/// 5 minutes in ms
static MX_NEGATIVE_TTL: AtomicUsize = AtomicUsize::new(300 * 1000);

/// The TTL for transient negative entries, in milliseconds.
static MX_TRANSIENT_NEGATIVE_TTL: AtomicUsize = AtomicUsize::new(30 * 1000);

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

pub fn set_mx_transient_negative_cache_ttl(duration: Duration) -> anyhow::Result<()> {
    let ms = duration
        .as_millis()
        .try_into()
        .context("set_mx_transient_negative_cache_ttl: duration is too large")?;
    MX_TRANSIENT_NEGATIVE_TTL.store(ms, Ordering::Relaxed);
    Ok(())
}

pub fn get_mx_transient_negative_ttl() -> Duration {
    Duration::from_millis(MX_TRANSIENT_NEGATIVE_TTL.load(Ordering::Relaxed) as u64)
}

/// Returns the cache lifetime for an `MX_CACHE` lookup result.
fn mx_cache_ttl(mx_result: &Result<Arc<MailExchanger>, MxResolveError>) -> Duration {
    match mx_result {
        Ok(mx) => match mx.expires {
            Some(exp) => exp
                .checked_duration_since(std::time::Instant::now())
                .unwrap_or_else(|| Duration::from_secs(10)),
            None => get_mx_negative_ttl(),
        },
        Err(err) => err.kind.negative_ttl(),
    }
}

struct ByPreference {
    pub hosts: Vec<String>,
    pub pref: u16,
    pub is_secure: bool,
    pub is_mx: bool,
}

async fn lookup_mx_record(
    domain_name: &Name,
    resolver: Option<&dyn Resolver>,
) -> Result<(Vec<ByPreference>, Instant), MxResolveError> {
    lookup_mx_record_limited(
        domain_name,
        resolver,
        &MX_CONCURRENCY_SEMA,
        get_mx_timeout(),
    )
    .await
}

/// A guard that counts one lookup in `MX_PERMIT_WAITERS` while it waits for an
/// MX concurrency permit. `Drop` removes it from the count, whether the
/// permit is acquired or the wait is abandoned by a timeout.
struct PermitWaitGuard;

impl PermitWaitGuard {
    fn enter() -> Self {
        MX_PERMIT_WAITERS.inc();
        Self
    }
}

impl Drop for PermitWaitGuard {
    fn drop(&mut self) {
        MX_PERMIT_WAITERS.dec();
    }
}

/// Perform the MX query for `domain_name`, bounding concurrency on
/// `concurrency` and the whole operation (permit wait plus query) on
/// `timeout_duration`.
async fn lookup_mx_record_limited(
    domain_name: &Name,
    resolver: Option<&dyn Resolver>,
    concurrency: &Semaphore,
    timeout_duration: Duration,
) -> Result<(Vec<ByPreference>, Instant), MxResolveError> {
    // Records whether the MX concurrency permit was obtained before the
    // timeout fired.
    let permit_acquired = AtomicBool::new(false);
    let mx_lookup = match timeout(timeout_duration, async {
        let wait_guard = PermitWaitGuard::enter();
        let _permit = concurrency.acquire().await;
        // Drop here ends the wait explicitly on success. If the enclosing
        // timeout instead drops this whole future first, the Drop of the guard
        // still runs and removes the count.
        drop(wait_guard);
        // Relaxed is enough: this store and the load after the match below
        // both happen in this function's own task, and no other task or
        // thread ever touches `permit_acquired`.
        permit_acquired.store(true, Ordering::Relaxed);
        match resolver {
            Some(r) => r.resolve(domain_name.clone(), RecordType::MX).await,
            None => {
                get_resolver()
                    .resolve(domain_name.clone(), RecordType::MX)
                    .await
            }
        }
    })
    .await
    {
        Ok(Ok(answer)) => answer,
        Ok(Err(dns_err)) => {
            return Err(MxResolveError {
                kind: classify_dns_error(&dns_err),
                message: dns_err.to_string(),
            });
        }
        Err(_elapsed) => {
            let (kind, message) = if permit_acquired.load(Ordering::Relaxed) {
                MX_QUERY_TIMEOUT.inc();
                (
                    MxResolveFailure::Indeterminate,
                    format!("MX query timed out after {timeout_duration:?}"),
                )
            } else {
                MX_PERMIT_TIMEOUT.inc();
                (
                    MxResolveFailure::NeverQueried,
                    format!(
                        "timed out after {timeout_duration:?} waiting for an MX concurrency \
                         permit; no query was sent"
                    ),
                )
            };
            return Err(MxResolveError { kind, message });
        }
    };
    let mx_records = mx_lookup.records;

    if mx_records.is_empty() {
        if mx_lookup.nxdomain {
            return Err(MxResolveError {
                kind: MxResolveFailure::NxDomain,
                message: "NXDOMAIN".to_string(),
            });
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

/// How resolving a domain's MX failed, classified by what the DNS exchange
/// established. A caller that needs to act on a specific case (such as
/// NXDOMAIN) can match on this after recovering the [`MxResolveError`] with
/// `downcast_ref`, rather than matching on the message text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MxResolveFailure {
    /// Authoritative negative response: the resolver reported that the name
    /// does not exist (NXDOMAIN). A resolver can return NXDOMAIN spuriously,
    /// while its own upstream is still propagating a new registration, and will
    /// stop doing so once that passes. A caller should re-check after a bounded
    /// negative-cache lifetime rather than treat this as final.
    NxDomain,
    /// Indeterminate negative response: a query was issued but did not produce
    /// an authoritative answer -- a timeout after the query was sent, a
    /// SERVFAIL or other failure RCODE, or a resolver I/O error. The true state
    /// of the name is unknown and a later query may resolve it.
    Indeterminate,
    /// No query was ever issued: the timeout elapsed while waiting for an MX
    /// concurrency permit. Indicates local concurrency pressure, nothing about
    /// the destination.
    NeverQueried,
    /// The MX set resolved, but the domain's MTA-STS enforce policy permits
    /// none of its own MX hosts. Mail to the domain cannot be delivered until
    /// its operator corrects the policy.
    PolicyRejected,
}

impl MxResolveFailure {
    /// Returns the negative-cache lifetime received by a failure of this kind.
    /// An authoritative NXDOMAIN or an operator-gated policy rejection holds
    /// for the full negative TTL. Indeterminate failures retry after a shorter
    /// interval. A lookup that was never issued returns a zero lifetime, which
    /// expires the cache entry immediately and makes the next caller
    /// re-resolve.
    fn negative_ttl(self) -> Duration {
        match self {
            MxResolveFailure::NxDomain | MxResolveFailure::PolicyRejected => get_mx_negative_ttl(),
            MxResolveFailure::Indeterminate => get_mx_transient_negative_ttl(),
            MxResolveFailure::NeverQueried => Duration::ZERO,
        }
    }
}

/// An MX resolution failure: its [`MxResolveFailure`] classification together
/// with a human-readable `message`. `Display` writes `message` verbatim, which
/// means `{err}` formatting (including `anyhow!("{err}")` and log output)
/// reproduces that text. Conversion to `anyhow::Error` keeps this concrete
/// type rather than erasing it: a caller holding the resulting `anyhow::Error`
/// can recover the classification with `downcast_ref::<MxResolveError>()`.
#[derive(Clone, Debug)]
pub struct MxResolveError {
    pub kind: MxResolveFailure,
    pub message: String,
}

impl MxResolveError {
    /// Whether the resolver returned NXDOMAIN for the name, for callers that
    /// need to single out that case without inspecting the message text.
    pub fn is_nxdomain(&self) -> bool {
        self.kind == MxResolveFailure::NxDomain
    }
}

impl std::fmt::Display for MxResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MxResolveError {}

/// Classify a resolver-level error. Both `DnsError` variants map to
/// `MxResolveFailure::Indeterminate` because neither contains an authoritative
/// answer.
fn classify_dns_error(err: &DnsError) -> MxResolveFailure {
    match err {
        // The query reached the resolver but came back without an
        // authoritative answer.
        DnsError::ResolveFailed(_) => MxResolveFailure::Indeterminate,
        // InvalidName is only ever constructed by the &str-parsing methods on
        // Resolver, not by resolve(Name, RecordType), which this function's
        // caller uses and which receives an already-parsed Name. The arm exists
        // for exhaustiveness and maps to Indeterminate as the conservative
        // default.
        DnsError::InvalidName(_) => MxResolveFailure::Indeterminate,
    }
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
    /// The applicable MTA-STS policy mode (`PolicyMode::None` when no policy
    /// applies). `hosts`/`by_pref` already exclude any hosts disallowed by the
    /// policy, so this is consulted only for TLS posture, not host gating.
    pub mta_sts: PolicyMode,
    #[serde(skip)]
    expires: Option<Instant>,
}

declare_cache! {
/// Caches domain name to computed set of MailExchanger records
static MX_CACHE: LruCacheWithTtl<(Name, Option<u16>), Result<Arc<MailExchanger>, MxResolveError>>::new("dns_resolver_mx", 64 * 1024);
}

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

declare_metric! {
/// Total number of MX lookups that timed out while waiting for an MX
/// concurrency permit before any query could be sent. Counts local
/// concurrency pressure, not any response from the destination.
static MX_PERMIT_TIMEOUT: IntCounter("dns_mx_resolve_permit_timeout");
}

declare_metric! {
/// Total number of MX lookups that timed out after the query was sent, while
/// waiting for the resolver to answer.
static MX_QUERY_TIMEOUT: IntCounter("dns_mx_resolve_query_timeout");
}

declare_metric! {
/// Number of MX lookups currently blocked waiting for an MX concurrency
/// permit before their query can be sent.
static MX_PERMIT_WAITERS: IntGauge("dns_mx_resolve_permit_waiters");
}

declare_metric! {
/// Total number of MailExchanger::resolve calls that failed because the
/// domain published an MTA-STS enforce policy that permits none of its own
/// MX hosts. Such domains are undeliverable until they fix their policy.
static MX_MTA_STS_IMPOSSIBLE: IntCounter("dns_mx_resolve_mta_sts_impossible");
}

/// The effect of an MTA-STS policy on a domain's resolved MX host set.
#[derive(Debug, PartialEq, Eq)]
enum StsEval {
    /// No host pruning required; record this TLS posture.
    Status(PolicyMode),
    /// Enforce policy with partial coverage: keep only the hosts whose
    /// index in the input is `true`.
    Prune(Vec<bool>),
    /// Enforce policy matches none of the hosts: the domain is undeliverable.
    Impossible,
}

/// Evaluate an MTA-STS policy against `hosts` (in resolution order, lowercased,
/// optionally `host:port`). Pure so it can be unit-tested without DNS/HTTP.
fn evaluate_mta_sts(hosts: &[String], policy: &MtaStsPolicy) -> StsEval {
    match policy.mode {
        PolicyMode::None => StsEval::Status(PolicyMode::None),
        PolicyMode::Testing => StsEval::Status(PolicyMode::Testing),
        PolicyMode::Enforce => {
            let matched: Vec<bool> = hosts
                .iter()
                .map(|h| {
                    let label = match has_colon_port(h) {
                        Some((label, _)) => label,
                        None => h.as_str(),
                    };
                    policy.mx_name_matches(label)
                })
                .collect();
            let match_count = matched.iter().filter(|m| **m).count();
            if match_count == 0 {
                StsEval::Impossible
            } else if match_count == hosts.len() {
                StsEval::Status(PolicyMode::Enforce)
            } else {
                StsEval::Prune(matched)
            }
        }
    }
}

/// Fetch and apply the MTA-STS policy for `name_fq` to the resolved MX set,
/// updating `by_pref`/`hosts`/`expires` in place. Returns the resolved policy
/// mode (when one applies), or `Err(message)` if the domain's enforce policy
/// permits none of its MX hosts and is therefore undeliverable.
async fn apply_mta_sts(
    name_fq: &Name,
    by_pref: &mut Vec<ByPreference>,
    hosts: &mut Vec<String>,
    expires: &mut Instant,
    resolver: Option<&dyn Resolver>,
) -> Result<PolicyMode, String> {
    let policy_domain = name_fq.to_ascii();
    let policy_domain = policy_domain.trim_end_matches('.');

    let policy = match mta_sts::get_policy_for_domain(policy_domain, resolver).await {
        Ok(policy) => policy,
        Err(err) => {
            // A transient fetch failure must not be treated as "impossible";
            // proceed as no-policy but re-attempt sooner than the full DNS TTL.
            tracing::debug!("MTA-STS policy fetch for {policy_domain} failed: {err:#}");
            *expires = (*expires).min(Instant::now() + MTA_STS_FETCH_RETRY);
            return Ok(PolicyMode::None);
        }
    };

    let mta_sts = match evaluate_mta_sts(hosts, &policy) {
        StsEval::Status(status) => status,
        StsEval::Prune(matched) => {
            // Partial coverage: prune the disallowed hosts so the site resolves
            // to only the permitted set (and rolls up only with others sharing
            // that set).
            let mut idx = 0;
            for pref in by_pref.iter_mut() {
                pref.hosts.retain(|_| {
                    let keep = matched[idx];
                    idx += 1;
                    keep
                });
            }
            by_pref.retain(|p| !p.hosts.is_empty());
            *hosts = by_pref
                .iter()
                .flat_map(|p| p.hosts.iter().cloned())
                .collect();
            PolicyMode::Enforce
        }
        StsEval::Impossible => {
            MX_MTA_STS_IMPOSSIBLE.inc();
            return Err(format!(
                "MTA-STS enforce policy for {policy_domain} permits none of its \
                 MX hosts {hosts:?}; allowed mx patterns: {patterns:?}. The \
                 destination is undeliverable until its MTA-STS policy is \
                 corrected.",
                patterns = policy.mx
            ));
        }
    };

    // Refresh in full: re-resolve when either the MX records or the policy
    // expire. max_age is clamped on parse, but guard the addition anyway so an
    // overflowing value expires immediately rather than panicking.
    let policy_expires = Instant::now()
        .checked_add(Duration::from_secs(policy.max_age))
        .unwrap_or_else(Instant::now);
    *expires = (*expires).min(policy_expires);
    Ok(mta_sts)
}

impl MailExchanger {
    pub async fn resolve(domain_name: &str) -> anyhow::Result<Arc<Self>> {
        Self::resolve_via(domain_name, None).await
    }

    /// Like [`resolve`](Self::resolve), but performs DNS via the supplied
    /// `resolver` when one is provided. A supplied resolver bypasses the shared
    /// MX cache so callers (such as tests using a fixture resolver) get
    /// hermetic, order-independent results.
    pub async fn resolve_via(
        domain_name: &str,
        resolver: Option<&dyn Resolver>,
    ) -> anyhow::Result<Arc<Self>> {
        MX_IN_PROGRESS.inc();
        let result = Self::resolve_impl(domain_name, resolver).await;
        MX_IN_PROGRESS.dec();
        if result.is_ok() {
            MX_SUCCESS.inc();
        } else {
            MX_FAIL.inc();
        }
        result
    }

    async fn resolve_impl(
        domain_name: &str,
        resolver: Option<&dyn Resolver>,
    ) -> anyhow::Result<Arc<Self>> {
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
                    mta_sts: PolicyMode::None,
                    expires: None,
                }));
            }
            DomainClassification::Domain(name_fq, opt_port) => (name_fq, opt_port),
        };

        // A supplied resolver bypasses the shared MX cache so results are
        // hermetic and order-independent.
        if resolver.is_some() {
            return Self::resolve_uncached(&name_fq, opt_port, domain_name, resolver)
                .await?
                .map_err(anyhow::Error::new);
        }

        let lookup_result = MX_CACHE
            .get_or_try_insert(
                &(name_fq.clone(), opt_port),
                mx_cache_ttl,
                Self::resolve_uncached(&name_fq, opt_port, domain_name, None),
            )
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;

        if !lookup_result.is_fresh {
            MX_CACHED.inc();
        }

        lookup_result.item.map_err(anyhow::Error::new)
    }

    async fn resolve_uncached(
        name_fq: &Name,
        opt_port: Option<u16>,
        domain_name: &str,
        resolver: Option<&dyn Resolver>,
    ) -> anyhow::Result<Result<Arc<MailExchanger>, MxResolveError>> {
        MX_QUERIES.inc();
        let start = Instant::now();
        let (mut by_pref, mut expires) = match lookup_mx_record(name_fq, resolver).await {
            Ok((by_pref, expires)) => (by_pref, expires),
            Err(err) => {
                let error = format!(
                    "MX lookup for {domain_name} failed after {elapsed:?}: {err}",
                    elapsed = start.elapsed()
                );
                tracing::debug!(
                    target: "mx_resolve",
                    domain = domain_name,
                    %error,
                    "MX lookup failed; domain drops out of any site_name rollup"
                );
                return Ok(Err(MxResolveError {
                    kind: err.kind,
                    message: error,
                }));
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

        // Evaluate MTA-STS against this domain's own resolution, before
        // site_name rollup. A domain whose enforce policy matches none of its
        // MX hosts is undeliverable and fails resolution so that it
        // self-isolates rather than affecting a shared site.
        let mta_sts = if is_mx && is_mta_sts_enabled() {
            match apply_mta_sts(name_fq, &mut by_pref, &mut hosts, &mut expires, resolver).await {
                Ok(status) => status,
                Err(error) => {
                    tracing::debug!(
                        target: "mx_resolve",
                        domain = domain_name,
                        %error,
                        "MTA-STS evaluation failed; domain drops out of any site_name rollup"
                    );
                    return Ok(Err(MxResolveError {
                        kind: MxResolveFailure::PolicyRejected,
                        message: error,
                    }));
                }
            }
        } else {
            PolicyMode::None
        };

        let by_pref = by_pref
            .into_iter()
            .map(|pref| (pref.pref, pref.hosts))
            .collect();

        let site_name = factor_names(&hosts);
        tracing::debug!(
            target: "mx_resolve",
            domain = domain_name,
            %site_name,
            ?hosts,
            is_mx,
            is_secure,
            elapsed_ms = start.elapsed().as_millis() as u64,
            "resolved MX to site_name"
        );
        let mx = Self {
            hosts,
            domain_name: name_fq.to_ascii(),
            site_name,
            by_pref,
            is_domain_literal: false,
            is_secure,
            is_mx,
            mta_sts,
            expires: Some(expires),
        };

        Ok(Ok(Arc::new(mx)))
    }

    pub fn has_expired(&self) -> bool {
        match self.expires {
            Some(deadline) => deadline <= Instant::now(),
            None => false,
        }
    }

    /// Returns the list of resolved MX hosts in *reverse* preference order.
    ///
    /// `max_addresses_per_host` caps the `A`/`AAAA` addresses kept from any
    /// single MX host, and `max_plan_size` caps the total across all hosts.
    /// Both bound the plan: a destination publishing a very large number of
    /// addresses cannot force an unbounded plan. When a cap truncates the
    /// list, the addresses that are dropped are from the least-preferred
    /// hosts.
    pub async fn resolve_addresses(
        &self,
        resolver: Option<&dyn Resolver>,
        strategy: IpLookupStrategy,
        max_plan_size: usize,
        max_addresses_per_host: usize,
    ) -> ResolvedMxAddresses {
        let mut result = vec![];

        // `by_pref` is a BTreeMap keyed by MX preference (lowest first);
        // iterating it in key order visits the most-preferred hosts first.
        'by_pref: for hosts in self.by_pref.values() {
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
                        addr,
                        is_secure: false,
                    });
                    continue;
                }

                match ip_lookup(mx_host, resolver, strategy).await {
                    Err(err) => {
                        tracing::error!("failed to resolve {mx_host}: {err:#}");
                        continue;
                    }
                    Ok((lookup, _expires)) => {
                        for addr in lookup.addrs.iter().take(max_addresses_per_host) {
                            let mut addr: HostOrSocketAddress = (*addr).into();
                            if let Some(port) = opt_port {
                                addr.set_port(port);
                            }
                            by_pref.push(ResolvedAddress {
                                name: mx_host.to_string(),
                                addr,
                                is_secure: lookup.secure,
                            });
                        }
                    }
                }
            }

            // Randomize the list of addresses within this preference
            // level. This probablistically "load balances" outgoing
            // traffic across MX hosts with equal preference value.
            {
                let mut rng = rand::thread_rng();
                by_pref.shuffle(&mut rng);
            }

            for addr in by_pref {
                if result.len() == max_plan_size {
                    // Stop resolving lower-preference hosts entirely, rather
                    // than resolving them and truncating afterward.
                    break 'by_pref;
                }
                result.push(addr);
            }
        }

        // Flip to the LIFO order the caller expects: the first candidate to
        // try ends up as the last element.
        result.reverse();
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

#[cfg(test)]
mod test {
    use super::*;
    use dns_resolver::{fully_qualify, TestResolver};

    fn policy(mode: &str, mx: &[&str]) -> MtaStsPolicy {
        let mut text = format!("version: STSv1\nmode: {mode}\nmax_age: 86400");
        for m in mx {
            text.push_str(&format!("\nmx: {m}"));
        }
        MtaStsPolicy::parse(&text).unwrap()
    }

    fn hosts(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn mta_sts_none_and_testing() {
        assert_eq!(
            evaluate_mta_sts(&hosts(&["mx01.mail.icloud.com."]), &policy("none", &[])),
            StsEval::Status(PolicyMode::None)
        );
        assert_eq!(
            evaluate_mta_sts(
                &hosts(&["mx01.mail.icloud.com."]),
                &policy("testing", &["*.mx.cloudflare.net"])
            ),
            StsEval::Status(PolicyMode::Testing)
        );
    }

    #[test]
    fn mta_sts_enforce_full_match() {
        assert_eq!(
            evaluate_mta_sts(
                &hosts(&["mx01.mail.icloud.com.", "mx02.mail.icloud.com."]),
                &policy("enforce", &["*.mail.icloud.com"])
            ),
            StsEval::Status(PolicyMode::Enforce)
        );
    }

    #[test]
    fn mta_sts_enforce_partial_prunes() {
        // Second host is not permitted; expect a prune mask, not failure.
        assert_eq!(
            evaluate_mta_sts(
                &hosts(&["mx01.mail.icloud.com.", "backup.example.net."]),
                &policy("enforce", &["*.mail.icloud.com"])
            ),
            StsEval::Prune(vec![true, false])
        );
    }

    #[test]
    fn mta_sts_enforce_impossible() {
        // The icloud-hosted random domain whose policy only allows cloudflare:
        // matches no host, so the domain is undeliverable.
        assert_eq!(
            evaluate_mta_sts(
                &hosts(&["mx01.mail.icloud.com.", "mx02.mail.icloud.com."]),
                &policy("enforce", &["*.mx.cloudflare.net"])
            ),
            StsEval::Impossible
        );
    }

    #[test]
    fn mta_sts_enforce_strips_port() {
        assert_eq!(
            evaluate_mta_sts(
                &hosts(&["mx01.mail.icloud.com.:587"]),
                &policy("enforce", &["*.mail.icloud.com"])
            ),
            StsEval::Status(PolicyMode::Enforce)
        );
    }

    #[tokio::test]
    async fn site_identity_does_not_reorder_connection_preferences() {
        let resolver = TestResolver::default()
            .with_zone("$ORIGIN priority-a.example.\n@ 600 MX 10 mx1.targets.test.\n@ 600 MX 20 mx2.targets.test.\n")
            .unwrap()
            .with_zone("$ORIGIN priority-b.example.\n@ 600 MX 10 mx2.targets.test.\n@ 600 MX 20 mx1.targets.test.\n")
            .unwrap();
        let a = MailExchanger::resolve_via("priority-a.example", Some(&resolver))
            .await
            .unwrap();
        let b = MailExchanger::resolve_via("priority-b.example", Some(&resolver))
            .await
            .unwrap();
        assert_eq!(a.site_name, b.site_name);
        assert_eq!(a.hosts, hosts(&["mx1.targets.test.", "mx2.targets.test."]));
        assert_eq!(b.hosts, hosts(&["mx2.targets.test.", "mx1.targets.test."]));
        assert_eq!(a.by_pref[&10], hosts(&["mx1.targets.test."]));
        assert_eq!(b.by_pref[&10], hosts(&["mx2.targets.test."]));
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
    mta_sts: None,
    expires: None,
}
"#
        );
        k9::snapshot!(
            v4_loopback
                .resolve_addresses(None, IpLookupStrategy::default(), 50, 10)
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
    mta_sts: None,
    expires: None,
}
"#
        );
        k9::snapshot!(
            v6_loopback_non_conforming
                .resolve_addresses(None, IpLookupStrategy::default(), 50, 10)
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
    mta_sts: None,
    expires: None,
}
"#
        );
        k9::snapshot!(
            v6_loopback
                .resolve_addresses(None, IpLookupStrategy::default(), 50, 10)
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

    fn fixture_resolver(zones: &[&str]) -> TestResolver {
        let mut resolver = TestResolver::default();
        for zone in zones {
            resolver = resolver.with_zone(zone).unwrap();
        }
        resolver
    }

    const GMAIL_ZONE: &str = r#"
$ORIGIN gmail.com.
@ 86400 MX 5 gmail-smtp-in.l.google.com.
@ 86400 MX 10 alt1.gmail-smtp-in.l.google.com.
@ 86400 MX 20 alt2.gmail-smtp-in.l.google.com.
@ 86400 MX 30 alt3.gmail-smtp-in.l.google.com.
@ 86400 MX 40 alt4.gmail-smtp-in.l.google.com.
"#;

    const GMAIL_HOSTS_ZONE: &str = r#"
$ORIGIN l.google.com.
gmail-smtp-in 300 A 142.251.2.26
alt1.gmail-smtp-in 300 A 108.177.104.27
alt2.gmail-smtp-in 300 A 74.125.126.27
alt3.gmail-smtp-in 300 A 172.253.113.26
alt4.gmail-smtp-in 300 A 173.194.77.27
"#;

    #[tokio::test]
    async fn lookup_gmail_mx() {
        let resolver = fixture_resolver(&[GMAIL_ZONE, GMAIL_HOSTS_ZONE]);
        let mut gmail = (*MailExchanger::resolve_via("gmail.com", Some(&resolver))
            .await
            .unwrap())
        .clone();
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
    mta_sts: None,
    expires: None,
}
"#
        );

        // The hosts are returned in reverse preference order (the last entry is
        // tried first). With one address per host the per-preference-level
        // shuffle in resolve_addresses is a no-op, so the order is stable.
        k9::snapshot!(
            gmail
                .resolve_addresses(Some(&resolver), IpLookupStrategy::Ipv4Only, 50, 10)
                .await,
            r#"
Addresses(
    [
        ResolvedAddress {
            name: "alt4.gmail-smtp-in.l.google.com.",
            addr: 173.194.77.27,
            is_secure: false,
        },
        ResolvedAddress {
            name: "alt3.gmail-smtp-in.l.google.com.",
            addr: 172.253.113.26,
            is_secure: false,
        },
        ResolvedAddress {
            name: "alt2.gmail-smtp-in.l.google.com.",
            addr: 74.125.126.27,
            is_secure: false,
        },
        ResolvedAddress {
            name: "alt1.gmail-smtp-in.l.google.com.",
            addr: 108.177.104.27,
            is_secure: false,
        },
        ResolvedAddress {
            name: "gmail-smtp-in.l.google.com.",
            addr: 142.251.2.26,
            is_secure: false,
        },
    ],
)
"#
        );
    }

    #[tokio::test]
    async fn resolve_addresses_caps_plan() {
        // One preference level with many hosts, each publishing a large A
        // RRset: 32 hosts x 32 addresses = 1024 candidates with no cap. A
        // lower-preference backup host with its own large RRset is dropped
        // entirely once the total cap is reached.
        let hosts = 32;
        let addrs_per_host = 32;
        let mut zone = String::from("$ORIGIN example.com.\n");
        for h in 0..hosts {
            zone.push_str(&format!("@ 86400 MX 10 host{h}.example.com.\n"));
        }
        zone.push_str("@ 86400 MX 20 backup.example.com.\n");
        for h in 0..hosts {
            for i in 1..=addrs_per_host {
                zone.push_str(&format!("host{h} 300 A 10.0.{h}.{i}\n"));
            }
        }
        for i in 1..=addrs_per_host {
            zone.push_str(&format!("backup 300 A 10.1.0.{i}\n"));
        }
        let resolver = fixture_resolver(&[&zone]);
        let mx = MailExchanger::resolve_via("example.com", Some(&resolver))
            .await
            .unwrap();

        // Without the caps the plan would hold all 1056 addresses, including
        // the lower-preference backup host.
        let uncapped = match mx
            .resolve_addresses(
                Some(&resolver),
                IpLookupStrategy::Ipv4Only,
                usize::MAX,
                usize::MAX,
            )
            .await
        {
            ResolvedMxAddresses::Addresses(a) => a,
            other => panic!("expected addresses, got {other:?}"),
        };
        k9::assert_equal!(uncapped.len(), (hosts + 1) * addrs_per_host);
        k9::assert_equal!(
            uncapped
                .iter()
                .filter(|a| a.name == "backup.example.com.")
                .count(),
            addrs_per_host
        );

        // The default caps bound the total to max_plan_size, a host
        // contributes no more than max_addresses_per_host, and the
        // addresses dropped are the least-preferred ones: the backup host
        // is absent entirely.
        let capped = match mx
            .resolve_addresses(Some(&resolver), IpLookupStrategy::Ipv4Only, 50, 10)
            .await
        {
            ResolvedMxAddresses::Addresses(a) => a,
            other => panic!("expected addresses, got {other:?}"),
        };
        k9::assert_equal!(capped.len(), 50);
        k9::assert_equal!(
            capped
                .iter()
                .filter(|a| a.name == "backup.example.com.")
                .count(),
            0
        );

        let mut per_host = std::collections::HashMap::new();
        for addr in &capped {
            *per_host.entry(addr.name.clone()).or_insert(0usize) += 1;
        }
        let max_from_one_host = per_host.values().copied().max().unwrap();
        assert!(
            max_from_one_host <= 10,
            "a single host contributed {max_from_one_host} addresses, exceeding the per-host cap"
        );
    }

    #[tokio::test]
    async fn lookup_punycode_no_mx_only_a() {
        let resolver = fixture_resolver(&[r#"
$ORIGIN xn--bb-eka.at.
@ 300 A 192.0.2.5
"#]);
        let mx = MailExchanger::resolve_via("xn--bb-eka.at", Some(&resolver))
            .await
            .unwrap();
        assert_eq!(mx.domain_name, "xn--bb-eka.at.");
        assert_eq!(mx.hosts[0], "xn--bb-eka.at.");
    }

    #[tokio::test]
    async fn lookup_nxdomain() {
        // The fixture has no zone covering this name, so the MX lookup is
        // NXDOMAIN.
        let resolver = fixture_resolver(&[]);
        let name = fully_qualify("not-mairs.aasland.com").unwrap();
        let err = match lookup_mx_record(&name, Some(&resolver)).await {
            Ok(_) => panic!("expected NXDOMAIN"),
            Err(err) => err,
        };
        k9::assert_equal!(err.to_string(), "NXDOMAIN");
        k9::assert_equal!(err.kind, MxResolveFailure::NxDomain);
        assert!(err.is_nxdomain());
    }

    #[tokio::test]
    async fn nxdomain_is_recoverable_through_anyhow() {
        // resolve_via converts the failure to anyhow. A caller must still be
        // able to recover the classification without parsing the message.
        let resolver = fixture_resolver(&[]);
        let err = MailExchanger::resolve_via("not-mairs.aasland.com", Some(&resolver))
            .await
            .unwrap_err();
        let mx_err = err
            .downcast_ref::<MxResolveError>()
            .expect("the typed MX error survives the anyhow conversion");
        assert!(mx_err.is_nxdomain());
        k9::assert_equal!(mx_err.kind, MxResolveFailure::NxDomain);
    }

    #[test]
    fn classify_resolver_error_is_indeterminate() {
        // ResolveFailed means the query reached the resolver without producing
        // an authoritative answer, which is the Indeterminate case.
        k9::assert_equal!(
            classify_dns_error(&DnsError::ResolveFailed("SERVFAIL".to_string())),
            MxResolveFailure::Indeterminate
        );
        // Production never calls classify_dns_error with InvalidName, but this
        // pins its fallback value (the same as ResolveFailed) so a future
        // caller that does pass one gets a deliberate answer instead of an
        // unreviewed one.
        k9::assert_equal!(
            classify_dns_error(&DnsError::InvalidName("bad".to_string())),
            MxResolveFailure::Indeterminate
        );
    }

    /// Resolver that sleeps for `delay` before failing, used to drive the
    /// query-timeout classification path.
    struct DelayResolver {
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl Resolver for DelayResolver {
        async fn resolve_ip(&self, _host: &str) -> Result<Vec<IpAddr>, DnsError> {
            unreachable!()
        }
        async fn resolve_mx(&self, _host: &str) -> Result<Vec<Name>, DnsError> {
            unreachable!()
        }
        async fn resolve_ptr(&self, _ip: IpAddr) -> Result<Vec<Name>, DnsError> {
            unreachable!()
        }
        async fn resolve(
            &self,
            _name: Name,
            _rrtype: RecordType,
        ) -> Result<dns_resolver::Answer, DnsError> {
            tokio::time::sleep(self.delay).await;
            Err(DnsError::ResolveFailed("delay elapsed".to_string()))
        }
    }

    #[tokio::test]
    async fn deadline_before_permit_is_never_queried() {
        // `acquire` blocks until a permit exists. With none in this semaphore
        // it blocks forever, leaving the 50ms timeout below as the only thing
        // that fires.
        let resolver = fixture_resolver(&[GMAIL_ZONE, GMAIL_HOSTS_ZONE]);
        let name = fully_qualify("gmail.com").unwrap();
        let no_permits = Semaphore::new(0);
        let before = MX_PERMIT_TIMEOUT.get();
        let err = match lookup_mx_record_limited(
            &name,
            Some(&resolver),
            &no_permits,
            Duration::from_millis(50),
        )
        .await
        {
            Ok(_) => panic!("expected a timeout before the permit was acquired"),
            Err(err) => err,
        };
        k9::assert_equal!(err.kind, MxResolveFailure::NeverQueried);
        k9::assert_equal!(
            err.message,
            "timed out after 50ms waiting for an MX concurrency permit; no query was sent"
        );
        k9::assert_equal!(MX_PERMIT_TIMEOUT.get() - before, 1);
    }

    #[tokio::test]
    async fn query_timeout_is_indeterminate() {
        let resolver = DelayResolver {
            delay: Duration::from_secs(60),
        };
        let name = fully_qualify("gmail.com").unwrap();
        // Unlike `deadline_before_permit_is_never_queried`, this semaphore
        // starts with a permit available. The query reaches `DelayResolver` and
        // is already in flight when the 50ms timeout elapses.
        let one_permit = Semaphore::new(1);
        let before = MX_QUERY_TIMEOUT.get();
        let err = match lookup_mx_record_limited(
            &name,
            Some(&resolver),
            &one_permit,
            Duration::from_millis(50),
        )
        .await
        {
            Ok(_) => panic!("expected the query to exceed the timeout"),
            Err(err) => err,
        };
        k9::assert_equal!(err.kind, MxResolveFailure::Indeterminate);
        k9::assert_equal!(err.message, "MX query timed out after 50ms");
        k9::assert_equal!(MX_QUERY_TIMEOUT.get() - before, 1);
    }

    #[test]
    fn negative_ttl_classifies_by_kind() {
        // Pins the per-kind mapping of MxResolveFailure::negative_ttl against
        // regression.
        k9::assert_equal!(
            MxResolveFailure::NxDomain.negative_ttl(),
            get_mx_negative_ttl()
        );
        k9::assert_equal!(
            MxResolveFailure::PolicyRejected.negative_ttl(),
            get_mx_negative_ttl()
        );
        k9::assert_equal!(
            MxResolveFailure::Indeterminate.negative_ttl(),
            get_mx_transient_negative_ttl()
        );
        k9::assert_equal!(
            MxResolveFailure::NeverQueried.negative_ttl(),
            Duration::ZERO
        );
    }

    /// Build a minimal successful `MailExchanger` for driving the cache
    /// directly. Leaves `expires` as `None`, which `mx_cache_ttl` treats as an
    /// unbounded success and caches for `get_mx_negative_ttl()`.
    fn fresh_mx(domain: &str) -> Arc<MailExchanger> {
        Arc::new(MailExchanger {
            domain_name: domain.to_string(),
            hosts: vec![format!("mx.{domain}.")],
            site_name: format!("mx.{domain}"),
            by_pref: BTreeMap::new(),
            is_domain_literal: false,
            is_secure: false,
            is_mx: true,
            mta_sts: PolicyMode::None,
            expires: None,
        })
    }

    #[tokio::test]
    async fn indeterminate_failure_cached_then_requeries() {
        // Calls get_or_try_insert on MX_CACHE directly, with mx_cache_ttl as
        // the real ttl_func, to exercise the negative-TTL selection in
        // mx_cache_ttl end to end. nextest runs each test in its own process,
        // and the shared MX_CACHE is private to this test.
        tokio::time::pause();
        let key = (fully_qualify("transient-ttl-test.invalid").unwrap(), None);
        let transient = get_mx_transient_negative_ttl();

        let first = MX_CACHE
            .get_or_try_insert(&key, mx_cache_ttl, async {
                Ok::<_, anyhow::Error>(Err(MxResolveError {
                    kind: MxResolveFailure::Indeterminate,
                    message: "SERVFAIL".to_string(),
                }))
            })
            .await
            .unwrap();
        assert!(first.is_fresh);
        assert!(first.item.is_err());

        // Before the transient TTL elapses the cached failure is returned.
        tokio::time::advance(transient / 2).await;
        let cached = MX_CACHE
            .get_or_try_insert(&key, mx_cache_ttl, async {
                // Panics if polled, which fails the test if the cache wrongly
                // re-queries instead of returning the cached failure.
                panic!("must not re-query before the transient TTL elapses");
                #[allow(unreachable_code)]
                Ok::<Result<Arc<MailExchanger>, MxResolveError>, anyhow::Error>(Ok(fresh_mx(
                    "transient-ttl-test.invalid",
                )))
            })
            .await
            .unwrap();
        assert!(!cached.is_fresh);
        assert!(cached.item.is_err());

        // Once it elapses the next lookup re-queries and can now succeed.
        tokio::time::advance(transient).await;
        let refreshed = MX_CACHE
            .get_or_try_insert(&key, mx_cache_ttl, async {
                Ok::<_, anyhow::Error>(Ok(fresh_mx("transient-ttl-test.invalid")))
            })
            .await
            .unwrap();
        assert!(refreshed.is_fresh);
        assert!(refreshed.item.is_ok());
    }

    #[tokio::test]
    async fn never_queried_failure_is_not_cached() {
        // A NeverQueried failure gets a zero TTL, which is already expired the
        // instant it is stored. The next lookup re-queries immediately, with no
        // time having to pass.
        let key = (fully_qualify("never-queried-test.invalid").unwrap(), None);

        let first = MX_CACHE
            .get_or_try_insert(&key, mx_cache_ttl, async {
                Ok::<_, anyhow::Error>(Err(MxResolveError {
                    kind: MxResolveFailure::NeverQueried,
                    message: "permit wait timed out".to_string(),
                }))
            })
            .await
            .unwrap();
        assert!(first.is_fresh);
        assert!(first.item.is_err());

        let refreshed = MX_CACHE
            .get_or_try_insert(&key, mx_cache_ttl, async {
                Ok::<_, anyhow::Error>(Ok(fresh_mx("never-queried-test.invalid")))
            })
            .await
            .unwrap();
        assert!(refreshed.is_fresh);
        assert!(refreshed.item.is_ok());
    }

    #[tokio::test]
    async fn lookup_null_mx() {
        let resolver = fixture_resolver(&[r#"
$ORIGIN example.com.
@ 3600 MX 0 .
"#]);
        let mut mx = (*MailExchanger::resolve_via("example.com", Some(&resolver))
            .await
            .unwrap())
        .clone();
        mx.expires.take();
        k9::snapshot!(
            &mx,
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
    is_secure: false,
    is_mx: true,
    mta_sts: None,
    expires: None,
}
"#
        );
    }

    #[tokio::test]
    async fn lookup_single_mx() {
        let resolver = fixture_resolver(&[r#"
$ORIGIN do.havedane.net.
@ 300 MX 10 do.havedane.net.
"#]);
        let mut mx = (*MailExchanger::resolve_via("do.havedane.net", Some(&resolver))
            .await
            .unwrap())
        .clone();
        mx.expires.take();
        k9::snapshot!(
            &mx,
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
    is_secure: false,
    is_mx: true,
    mta_sts: None,
    expires: None,
}
"#
        );
    }

    #[tokio::test]
    async fn mx_lookup_no_mx_falls_back_to_a() {
        // The zone exists (so the lookup is not NXDOMAIN) but has no MX record
        // for www, so resolution falls back to the domain's own A record.
        let resolver = fixture_resolver(&[r#"
$ORIGIN example.com.
www 300 A 192.0.2.1
"#]);
        let mut mx = (*MailExchanger::resolve_via("www.example.com", Some(&resolver))
            .await
            .unwrap())
        .clone();
        mx.expires.take();
        k9::snapshot!(
            &mx,
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
    mta_sts: None,
    expires: None,
}
"#
        );
    }
}
