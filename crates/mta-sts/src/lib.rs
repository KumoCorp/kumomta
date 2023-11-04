use futures::future::BoxFuture;
use hickory_resolver::Name;
use lruttl::LruCacheWithTtl;
use once_cell::sync::Lazy;
use policy::MtaStsPolicy;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static CACHE: Lazy<Mutex<LruCacheWithTtl<Name, CachedPolicy>>> =
    Lazy::new(|| Mutex::new(LruCacheWithTtl::new(64 * 1024)));

pub mod dns;
pub mod policy;

#[derive(Clone)]
struct CachedPolicy {
    pub id: String,
    pub policy: Arc<MtaStsPolicy>,
}

struct Getter {}

impl policy::Get for Getter {
    fn http_get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let response = reqwest::Client::builder()
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

pub async fn get_policy_for_domain(policy_domain: &str) -> anyhow::Result<Arc<MtaStsPolicy>> {
    let resolver = dns_resolver::get_resolver();
    get_policy_for_domain_impl(policy_domain, &*resolver, &Getter {}).await
}

async fn get_policy_for_domain_impl(
    policy_domain: &str,
    resolver: &dyn dns::Lookup,
    getter: &dyn policy::Get,
) -> anyhow::Result<Arc<MtaStsPolicy>> {
    let name = Name::from_str_relaxed(policy_domain)?.to_lowercase();

    if let Some(cached) = CACHE.lock().unwrap().get(&name) {
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

    let expires = Instant::now() + Duration::from_secs(policy.max_age);

    CACHE.lock().unwrap().insert(
        name,
        CachedPolicy {
            id: record.id,
            policy: Arc::clone(&policy),
        },
        expires,
    );

    Ok(policy)
}

/*
#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn get_gmail_policy() {
        k9::snapshot!(
            get_policy_for_domain("gmail.com").await.unwrap(),
            r#"
MtaStsPolicy {
    mode: Enforce,
    mx: [
        "gmail-smtp-in.l.google.com",
        "*.gmail-smtp-in.l.google.com",
    ],
    max_age: 86400,
    fields: {},
}
"#
        );
    }
}
*/
