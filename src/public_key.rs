use rsa::{pkcs8, RsaPublicKey};
use slog::{debug, warn};
use std::collections::HashMap;
use std::sync::Arc;

use crate::{dns, parser, DKIMError, DNS_NAMESPACE};

// https://datatracker.ietf.org/doc/html/rfc6376#section-6.1.2
pub(crate) async fn retrieve_public_key(
    logger: &slog::Logger,
    resolver: Arc<dyn dns::Lookup>,
    domain: String,
    subdomain: String,
    key_type: Option<String>,
) -> Result<RsaPublicKey, DKIMError> {
    let dns_name = format!("{}.{}.{}", subdomain, DNS_NAMESPACE, domain);
    let res = resolver.lookup_txt(&dns_name).await?;
    // TODO: Return multiple keys for when verifiying the signatures. During key
    // rotation they are often multiple keys to consider.
    let txt = res.first().ok_or(DKIMError::NoKeyForSignature)?;
    debug!(logger, "DKIM TXT: {:?}", txt);

    // Parse the tags inside the DKIM TXT DNS record
    let (_, tags) = parser::tag_list(txt).map_err(|err| {
        warn!(logger, "key syntax error: {}", err);
        DKIMError::KeySyntaxError
    })?;

    let mut tags_map = HashMap::new();
    for tag in &tags {
        tags_map.insert(tag.name.clone(), tag.clone());
    }

    // Check version
    if let Some(version) = tags_map.get("v") {
        if version.value != "DKIM1" {
            return Err(DKIMError::KeyIncompatibleVersion);
        }
    }

    // Check key has right type
    if let Some(v) = tags_map.get("k") {
        let key_type = key_type.unwrap_or_else(|| "rsa".to_string());
        if v.value != key_type {
            return Err(DKIMError::InappropriateKeyAlgorithm);
        }
    }

    let tag = tags_map.get("p").ok_or(DKIMError::NoKeyForSignature)?;
    let bytes = base64::decode(&tag.value).map_err(|err| {
        DKIMError::KeyUnavailable(format!("failed to decode public key: {}", err))
    })?;
    let key = pkcs8::DecodePublicKey::from_public_key_der(&bytes)
        .map_err(|err| DKIMError::KeyUnavailable(format!("failed to parse public key: {}", err)))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::BoxFuture;

    #[tokio::test]
    async fn test_retrieve_public_key() {
        struct TestResolver {}
        impl dns::Lookup for TestResolver {
            fn lookup_txt<'a>(
                &'a self,
                name: &'a str,
            ) -> BoxFuture<'a, Result<Vec<String>, DKIMError>> {
                Box::pin(async move {
                    assert_eq!(name, "dkim._domainkey.cloudflare.com");
                    Ok(vec!["v=DKIM1; k=rsa; p=MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA6gmVDBSBJ0l1/33uAF0gwIsrjQV6nnYjL9DMX6+ez4NNJ2um0InYy128Rd+OlIhmdSld6g3tj3O6R+BwsYsQgU8RWE8VJaRybvPw2P3Asgms4uPrFWHSFiWMPH0P9i/oPwnUO9jZKHiz4+MzFC3bG8BacX7YIxCuWnDU8XNmNsRaLmrv9CHX4/3GHyoHSmDA1ETtyz9JHRCOC8ho8C7b4f2Auwedlau9Lid9LGBhozhgRFhrFwFMe93y34MO1clPbY6HwxpudKWBkMQCTlmXVRnkKxHlJ+fYCyC2jjpCIbGWj2oLxBtFOASWMESR4biW0ph2bsZXslcUSPMTVTkFxQIDAQAB".to_string()])
                })
            }
        }
        let resolver = Arc::new(TestResolver {});
        let logger = slog::Logger::root(slog::Discard, slog::o!());

        retrieve_public_key(
            &logger,
            resolver,
            "cloudflare.com".to_string(),
            "dkim".to_string(),
            None,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_retrieve_public_key_incompatible_version() {
        struct TestResolver {}
        impl dns::Lookup for TestResolver {
            fn lookup_txt<'a>(
                &'a self,
                name: &'a str,
            ) -> BoxFuture<'a, Result<Vec<String>, DKIMError>> {
                Box::pin(async move {
                    assert_eq!(name, "dkim._domainkey.cloudflare.com");
                    Ok(vec!["v=DKIM6; p=key".to_string()])
                })
            }
        }
        let resolver = Arc::new(TestResolver {});
        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let key = retrieve_public_key(
            &logger,
            resolver,
            "cloudflare.com".to_string(),
            "dkim".to_string(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(key, DKIMError::KeyIncompatibleVersion);
    }

    #[tokio::test]
    async fn test_retrieve_public_key_inappropriate_key_algorithm() {
        struct TestResolver {}
        impl dns::Lookup for TestResolver {
            fn lookup_txt<'a>(
                &'a self,
                name: &'a str,
            ) -> BoxFuture<'a, Result<Vec<String>, DKIMError>> {
                Box::pin(async move {
                    assert_eq!(name, "dkim._domainkey.cloudflare.com");
                    Ok(vec!["v=DKIM1; p=key; k=foo".to_string()])
                })
            }
        }
        let resolver = Arc::new(TestResolver {});
        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let key = retrieve_public_key(
            &logger,
            resolver,
            "cloudflare.com".to_string(),
            "dkim".to_string(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(key, DKIMError::InappropriateKeyAlgorithm);
    }
}
