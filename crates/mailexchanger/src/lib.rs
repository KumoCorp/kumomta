use anyhow::Context;
use dns_resolver::{
    fully_qualify, get_resolver, has_colon_port, ip_lookup, DomainClassification, IpLookupStrategy,
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

struct ByPreference {
    pub hosts: Vec<String>,
    pub pref: u16,
    pub is_secure: bool,
    pub is_mx: bool,
}

async fn lookup_mx_record(
    domain_name: &Name,
    resolver: Option<&dyn Resolver>,
) -> anyhow::Result<(Vec<ByPreference>, Instant)> {
    let mx_lookup = timeout(get_mx_timeout(), async {
        let _permit = MX_CONCURRENCY_SEMA.acquire().await;
        match resolver {
            Some(r) => r.resolve(domain_name.clone(), RecordType::MX).await,
            None => {
                get_resolver()
                    .resolve(domain_name.clone(), RecordType::MX)
                    .await
            }
        }
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
    /// The applicable MTA-STS policy mode, when one was resolved.
    /// `hosts`/`by_pref` already exclude any hosts disallowed by the
    /// policy, so this is consulted only for TLS posture, not host gating.
    pub mta_sts: Option<PolicyMode>,
    #[serde(skip)]
    expires: Option<Instant>,
}

declare_cache! {
/// Caches domain name to computed set of MailExchanger records
static MX_CACHE: LruCacheWithTtl<(Name, Option<u16>), Result<Arc<MailExchanger>, String>>::new("dns_resolver_mx", 64 * 1024);
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
/// Total number of MailExchanger::resolve calls that failed because the
/// domain published an MTA-STS enforce policy that permits none of its own
/// MX hosts. Such domains are undeliverable until they fix their policy.
static MX_MTA_STS_IMPOSSIBLE: IntCounter("dns_mx_resolve_mta_sts_impossible");
}

/// The effect of an MTA-STS policy on a domain's resolved MX host set.
#[derive(Debug, PartialEq, Eq)]
enum StsEval {
    /// No host pruning required; record this TLS status (if any).
    Status(Option<PolicyMode>),
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
        PolicyMode::None => StsEval::Status(None),
        PolicyMode::Testing => StsEval::Status(Some(PolicyMode::Testing)),
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
                StsEval::Status(Some(PolicyMode::Enforce))
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
) -> Result<Option<PolicyMode>, String> {
    let policy_domain = name_fq.to_ascii();
    let policy_domain = policy_domain.trim_end_matches('.');

    let policy = match mta_sts::get_policy_for_domain(policy_domain, resolver).await {
        Ok(policy) => policy,
        Err(err) => {
            // A transient fetch failure must not be treated as "impossible";
            // proceed as no-policy but re-attempt sooner than the full DNS TTL.
            tracing::debug!("MTA-STS policy fetch for {policy_domain} failed: {err:#}");
            *expires = (*expires).min(Instant::now() + MTA_STS_FETCH_RETRY);
            return Ok(None);
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
            Some(PolicyMode::Enforce)
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

    // Refresh holistically: re-resolve when either the MX records or the
    // policy expire.
    *expires = (*expires).min(Instant::now() + Duration::from_secs(policy.max_age));
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
                    mta_sts: None,
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
                .map_err(|err| anyhow::anyhow!("{err}"));
        }

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
                Self::resolve_uncached(&name_fq, opt_port, domain_name, None),
            )
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;

        if !lookup_result.is_fresh {
            MX_CACHED.inc();
        }

        lookup_result.item.map_err(|err| anyhow::anyhow!("{err}"))
    }

    async fn resolve_uncached(
        name_fq: &Name,
        opt_port: Option<u16>,
        domain_name: &str,
        resolver: Option<&dyn Resolver>,
    ) -> anyhow::Result<Result<Arc<MailExchanger>, String>> {
        MX_QUERIES.inc();
        let start = Instant::now();
        let (mut by_pref, mut expires) = match lookup_mx_record(name_fq, resolver).await {
            Ok((by_pref, expires)) => (by_pref, expires),
            Err(err) => {
                let error = format!(
                    "MX lookup for {domain_name} failed after {elapsed:?}: {err:#}",
                    elapsed = start.elapsed()
                );
                return Ok(Err(error));
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
                Err(error) => return Ok(Err(error)),
            }
        } else {
            None
        };

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

    /// Returns the list of resolve MX hosts in *reverse* preference
    /// order; the first one to try is the last element.
    /// smtp_dispatcher.rs relies on this ordering, as it will pop
    /// off candidates until it has exhausted its connection plan.
    pub async fn resolve_addresses(
        &self,
        resolver: Option<&dyn Resolver>,
        strategy: IpLookupStrategy,
    ) -> ResolvedMxAddresses {
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

                match ip_lookup(mx_host, resolver, strategy).await {
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

#[cfg(test)]
mod test {
    use super::*;
    use dns_resolver::TestResolver;

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
            StsEval::Status(None)
        );
        assert_eq!(
            evaluate_mta_sts(
                &hosts(&["mx01.mail.icloud.com."]),
                &policy("testing", &["*.mx.cloudflare.net"])
            ),
            StsEval::Status(Some(PolicyMode::Testing))
        );
    }

    #[test]
    fn mta_sts_enforce_full_match() {
        assert_eq!(
            evaluate_mta_sts(
                &hosts(&["mx01.mail.icloud.com.", "mx02.mail.icloud.com."]),
                &policy("enforce", &["*.mail.icloud.com"])
            ),
            StsEval::Status(Some(PolicyMode::Enforce))
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
            StsEval::Status(Some(PolicyMode::Enforce))
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
    mta_sts: None,
    expires: None,
}
"#
        );
        k9::snapshot!(
            v4_loopback
                .resolve_addresses(None, IpLookupStrategy::default())
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
                .resolve_addresses(None, IpLookupStrategy::default())
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
                .resolve_addresses(None, IpLookupStrategy::default())
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
                .resolve_addresses(Some(&resolver), IpLookupStrategy::Ipv4Only)
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
}
