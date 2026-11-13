use dns_resolver::Resolver;
use futures::future::BoxFuture;
use hickory_resolver::proto::rr::Name;
use policy::MtaStsPolicy;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

lruttl::declare_cache! {
/// Caches MTA-STS policy information by domain
static CACHE: LruCacheWithTtl<Name, CachedPolicy>::new("mta_sts_policy", 64 * 1024);
}

pub mod dns;
pub mod policy;

#[derive(Clone, Debug)]
struct CachedPolicy {
    pub id: String,
    pub policy: Arc<MtaStsPolicy>,
}

struct Getter {}

impl policy::Get for Getter {
    fn http_get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let response = kumo_http_client::builder()
                // <https://datatracker.ietf.org/doc/html/rfc8461#section-3.3>
                // HTTP 3xx redirects MUST NOT be followed
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(20))
                .build()?
                .request(reqwest::Method::GET, url)
                .send()
                .await?;

            // <https://datatracker.ietf.org/doc/html/rfc8461#section-3.3>
            // Policies fetched via HTTPS are only valid if the HTTP
            // response code is 200 (OK)
            let status = response.status();
            if status != reqwest::StatusCode::OK {
                anyhow::bail!("failed to GET {url}: {status}");
            }

            // <https://datatracker.ietf.org/doc/html/rfc8461#section-3.2>
            // senders SHOULD validate that the media type is "text/plain"
            // to guard against cases where web servers allow untrusted users
            // to host non-text content.
            // We need to do some manual grubbing about for this, as reqwest's
            // Response::text() method doesn't verify that the type is textual,
            // just whether it decodes as text, which is precisely what we're
            // trying to guard against.

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .ok_or_else(|| anyhow::anyhow!("missing required Content-Type header"))?;

            let content_type = content_type.to_str()?;

            let ct = if let Some((ct, _)) = content_type.split_once(';') {
                ct.trim()
            } else {
                content_type.trim()
            };
            if ct != "text/plain" {
                anyhow::bail!("Content-Type must be text/plain, got {content_type}");
            }

            Ok(response.text().await?)
        })
    }
}

/// Test hook. When set, [`get_policy_for_domain`] serves policies from this
/// fixed map instead of performing live DNS TXT + HTTPS fetching. Each fetched
/// test policy populates the real policy cache, with its max_age, but bypasses
/// cache revalidation. A domain absent from the map resolves to an error, exactly
/// like a domain that publishes no MTA-STS record.
static TEST_POLICIES: Mutex<Option<Arc<BTreeMap<String, Arc<MtaStsPolicy>>>>> = Mutex::new(None);

/// Install a fixed set of MTA-STS policies, keyed by policy domain (no trailing
/// dot), for use in tests. While installed, all policy resolution is served
/// from this map without touching the network.
pub fn set_test_policies(policies: BTreeMap<String, MtaStsPolicy>) {
    let map = policies
        .into_iter()
        .map(|(domain, policy)| (domain, Arc::new(policy)))
        .collect();
    *TEST_POLICIES.lock().unwrap() = Some(Arc::new(map));
}

pub async fn get_policy_for_domain(
    policy_domain: &str,
    resolver: Option<&dyn Resolver>,
) -> anyhow::Result<Arc<MtaStsPolicy>> {
    let test_policies = TEST_POLICIES.lock().unwrap().clone();
    if let Some(policies) = test_policies {
        let domain = policy_domain.trim_end_matches('.');
        let policy = policies
            .get(domain)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no MTA-STS policy for {domain}"))?;
        let name = Name::from_str_relaxed(domain)?.to_lowercase();
        cache_policy(name, "test".to_string(), Arc::clone(&policy)).await;
        return Ok(policy);
    }

    match resolver {
        Some(resolver) => get_policy_for_domain_impl(policy_domain, resolver, &Getter {}).await,
        None => {
            let resolver = dns_resolver::get_resolver();
            get_policy_for_domain_impl(policy_domain, &**resolver, &Getter {}).await
        }
    }
}

fn cache_lookup(name: &Name) -> Option<CachedPolicy> {
    CACHE.get(name)
}

/// Returns an unexpired cached policy without DNS or HTTPS requests. A cache
/// miss (including an expired entry) does not establish that a domain has no
/// policy. Reading an entry does not extend its lifetime.
pub fn get_cached_policy_for_domain(policy_domain: &str) -> Option<Arc<MtaStsPolicy>> {
    let name = Name::from_str_relaxed(policy_domain.trim_end_matches('.'))
        .ok()?
        .to_lowercase();
    cache_lookup(&name).map(|cached| cached.policy)
}

async fn get_policy_for_domain_impl(
    policy_domain: &str,
    resolver: &dyn Resolver,
    getter: &dyn policy::Get,
) -> anyhow::Result<Arc<MtaStsPolicy>> {
    let name = Name::from_str_relaxed(policy_domain.trim_end_matches('.'))?.to_lowercase();

    if let Some(cached) = cache_lookup(&name) {
        // Removal of the DNS record does not invalidate our
        // cached result, only updating it with a different id
        let still_valid = dns::resolve_dns_record(policy_domain, resolver)
            .await
            .map(|r| cached.id == r.id)
            .unwrap_or(true);

        if still_valid {
            return Ok(Arc::clone(&cached.policy));
        }
    }

    let record = dns::resolve_dns_record(policy_domain, resolver).await?;

    let policy = Arc::new(policy::load_policy_for_domain(policy_domain, getter).await?);
    cache_policy(name, record.id, Arc::clone(&policy)).await;
    Ok(policy)
}

async fn cache_policy(name: Name, id: String, policy: Arc<MtaStsPolicy>) {
    // max_age is clamped on parse, but guard the addition anyway: a value that
    // overflowed the monotonic clock would otherwise panic and take the process
    // down. On overflow, expire immediately and re-fetch next lookup.
    let expires = Instant::now()
        .checked_add(Duration::from_secs(policy.max_age))
        .unwrap_or_else(Instant::now);

    CACHE
        .insert(name, CachedPolicy { id, policy }, expires.into())
        .await;
}

#[cfg(test)]
mod test {
    use super::*;
    use dns_resolver::TestResolver;
    use policy::test::TestGetter;
    use policy::PolicyMode;

    #[tokio::test]
    async fn cached_policy_survives_dns_failure() {
        let domain = "cached-enforce.example.com";
        let resolver = TestResolver::default()
            .with_txt("_mta-sts.cached-enforce.example.com", "v=STSv1; id=one;");
        let getter = TestGetter::new([(
            "https://mta-sts.cached-enforce.example.com/.well-known/mta-sts.txt",
            "version: STSv1\nmode: enforce\nmx: mail.example.com\nmax_age: 86400",
        )]);
        assert!(get_cached_policy_for_domain(domain).is_none());

        // Exercise production parsing and cache insertion, not TEST_POLICIES.
        // Only DNS and the authenticated HTTP response body are simulated.
        let fetched = get_policy_for_domain_impl(domain, &resolver, &getter)
            .await
            .unwrap();
        let name = Name::from_str_relaxed(domain).unwrap();
        let expiration = CACHE.lookup(&name).unwrap().expiration;
        let cached = get_cached_policy_for_domain("CACHED-ENFORCE.EXAMPLE.COM.").unwrap();
        assert!(Arc::ptr_eq(&fetched, &cached));
        assert_eq!(cached.mode, PolicyMode::Enforce);
        assert!(cached.mx_name_matches("mail.example.com."));

        let failed_resolver =
            TestResolver::default().with_servfail("_mta-sts.cached-enforce.example.com");
        let unavailable = TestGetter::new([]);
        let retained = get_policy_for_domain_impl(
            "cached-enforce.example.com.",
            &failed_resolver,
            &unavailable,
        )
        .await
        .unwrap();
        assert!(Arc::ptr_eq(&cached, &retained));
        assert!(Arc::ptr_eq(
            &cached,
            &get_cached_policy_for_domain(domain).unwrap()
        ));
        assert_eq!(CACHE.lookup(&name).unwrap().expiration, expiration);
    }

    #[tokio::test]
    async fn cached_policy_expires_without_extending_lifetime() {
        let domain = "expiring-enforce.example.com";
        let resolver = TestResolver::default()
            .with_txt("_mta-sts.expiring-enforce.example.com", "v=STSv1; id=one;");
        let getter = TestGetter::new([(
            "https://mta-sts.expiring-enforce.example.com/.well-known/mta-sts.txt",
            "version: STSv1\nmode: enforce\nmx: mail.example.com\nmax_age: 1",
        )]);
        get_policy_for_domain_impl(domain, &resolver, &getter)
            .await
            .unwrap();
        let name = Name::from_str_relaxed(domain).unwrap();
        let expiration = CACHE.lookup(&name).unwrap().expiration;
        assert!(get_cached_policy_for_domain(domain).is_some());
        assert_eq!(CACHE.lookup(&name).unwrap().expiration, expiration);

        // The real cache uses an absolute deadline: reads must not turn a
        // short-lived enforce policy into an indefinitely retained policy.
        tokio::time::sleep_until(expiration).await;
        assert!(get_cached_policy_for_domain(domain).is_none());
        assert!(
            get_policy_for_domain_impl(domain, &resolver, &TestGetter::new([]))
                .await
                .is_err()
        );
        assert!(get_cached_policy_for_domain(domain).is_none());
    }

    #[tokio::test]
    async fn cached_policy_zero_max_age_is_not_retained() {
        let domain = "zero-age.example.com";
        let resolver =
            TestResolver::default().with_txt("_mta-sts.zero-age.example.com", "v=STSv1; id=one;");
        let getter = TestGetter::new([(
            "https://mta-sts.zero-age.example.com/.well-known/mta-sts.txt",
            "version: STSv1\nmode: enforce\nmx: mail.example.com\nmax_age: 0",
        )]);
        let fetched = get_policy_for_domain_impl(domain, &resolver, &getter)
            .await
            .unwrap();
        assert_eq!(fetched.mode, PolicyMode::Enforce);
        assert!(get_cached_policy_for_domain(domain).is_none());
    }

    #[test]
    fn cached_policy_miss_does_not_discover_policy() {
        assert!(get_cached_policy_for_domain("uncached.example.com").is_none());
        assert!(get_cached_policy_for_domain("invalid..example.com").is_none());
    }
}
