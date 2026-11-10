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
/// fixed map instead of performing live DNS TXT + HTTPS fetching. A domain
/// absent from the map resolves to an error, exactly like a domain that
/// publishes no MTA-STS record.
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
    if let Some(policies) = TEST_POLICIES.lock().unwrap().clone() {
        let domain = policy_domain.trim_end_matches('.');
        return policies
            .get(domain)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no MTA-STS policy for {domain}"));
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

async fn get_policy_for_domain_impl(
    policy_domain: &str,
    resolver: &dyn Resolver,
    getter: &dyn policy::Get,
) -> anyhow::Result<Arc<MtaStsPolicy>> {
    let name = Name::from_str_relaxed(policy_domain)?.to_lowercase();

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

    // max_age is clamped on parse, but guard the addition anyway: a value that
    // overflowed the monotonic clock would otherwise panic and take the process
    // down. On overflow, expire immediately and re-fetch next lookup.
    let expires = Instant::now()
        .checked_add(Duration::from_secs(policy.max_age))
        .unwrap_or_else(Instant::now);

    CACHE
        .insert(
            name,
            CachedPolicy {
                id: record.id,
                policy: Arc::clone(&policy),
            },
            expires.into(),
        )
        .await;

    Ok(policy)
}

#[cfg(test)]
mod test {
    use super::*;
    use dns_resolver::TestResolver;
    use hickory_resolver::proto::op::ResponseCode;
    use hickory_resolver::proto::rr::RecordType;
    use policy::test::TestGetter;
    use policy::PolicyMode;

    /// Exercise the real cache, not set_test_policies. TestGetter stands in for
    /// a successful authenticated HTTPS fetch; it does not test authentication.
    /// MX routing and SMTP enforcement are covered by the integration tests.
    #[tokio::test]
    async fn cached_enforce_survives_mx_and_txt_nodata() {
        let domain = "cached-implicit-mx.example";
        let name = Name::from_str_relaxed(domain).unwrap();
        let before = TestResolver::default()
            .with_zone(
                r#"
$ORIGIN cached-implicit-mx.example.
@        60 MX 10 mail.cached-implicit-mx.example.
_mta-sts 60 TXT "v=STSv1; id=original;"
"#,
            )
            .unwrap();
        let getter = TestGetter::new([(
            "https://mta-sts.cached-implicit-mx.example/.well-known/mta-sts.txt",
            "version: STSv1\nmode: enforce\nmx: mail.cached-implicit-mx.example\nmax_age: 3600",
        )]);
        assert!(cache_lookup(&name).is_none());
        assert!(!before.resolve_mx(domain).await.unwrap().is_empty());
        let original = get_policy_for_domain_impl(domain, &before, &getter)
            .await
            .unwrap();

        // Model the DNS answers after the MX cache expires: accepted, unsigned
        // NODATA, with usable addresses and no TXT discovery record. This must
        // not discard the still-unexpired authenticated policy.
        let after = TestResolver::default()
            .with_zone(
                r#"
$ORIGIN cached-implicit-mx.example.
@ 60 A    192.0.2.1
@ 60 AAAA 2001:db8::1
"#,
            )
            .unwrap();
        let mx = after.resolve(name.clone(), RecordType::MX).await.unwrap();
        assert_eq!(mx.response_code, ResponseCode::NoError);
        assert!(mx.records.is_empty());
        assert!(!mx.nxdomain && !mx.secure && !mx.bogus);
        assert!(!after.resolve_ip(domain).await.unwrap().is_empty());
        assert!(dns::resolve_dns_record(domain, &after).await.is_err());
        assert!(
            cache_lookup(&name).is_some(),
            "policy must still be unexpired"
        );

        // Use the public lookup with its real Getter. Without the cached
        // policy this fails on missing TXT, before any network HTTP request.
        let retained = get_policy_for_domain(domain, Some(&after)).await.unwrap();
        assert!(Arc::ptr_eq(&original, &retained));
        assert_eq!(retained.mode, PolicyMode::Enforce);
        assert_eq!(retained.max_age, 3600);
        assert!(retained.mx_name_matches("mail.cached-implicit-mx.example."));
        assert!(!retained.mx_name_matches("cached-implicit-mx.example."));
    }
}
