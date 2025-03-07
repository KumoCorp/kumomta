use crate::{parser, DKIMError, DkimPublicKey, DNS_NAMESPACE};
use dns_resolver::Resolver;
use openssl::pkey::PKey;
use openssl::rsa::Rsa;
use std::collections::HashMap;

const RSA_KEY_TYPE: &str = "rsa";
const ED25519_KEY_TYPE: &str = "ed25519";

enum KeyType {
    Rsa,
    Ed25519,
}

impl KeyType {
    fn parse(k: &str) -> Result<Self, DKIMError> {
        match k {
            RSA_KEY_TYPE => Ok(Self::Rsa),
            ED25519_KEY_TYPE => Ok(Self::Ed25519),
            _ => Err(DKIMError::InappropriateKeyAlgorithm),
        }
    }
}

fn parse_single_key(txt: &str) -> Result<DkimPublicKey, DKIMError> {
    tracing::debug!("DKIM TXT: {:?}", txt);

    // Parse the tags inside the DKIM TXT DNS record
    let (_, tags) = parser::tag_list(txt).map_err(|err| {
        tracing::warn!("key syntax error: {}", err);
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

    // Get key type
    let key_type = tags_map
        .get("k")
        .map(|v| v.value.as_str())
        .unwrap_or(RSA_KEY_TYPE);
    let key_type = KeyType::parse(key_type)?;

    let tag = tags_map.get("p").ok_or(DKIMError::NoKeyForSignature)?;
    let bytes = data_encoding::BASE64
        .decode(tag.value.as_bytes())
        .map_err(|err| {
            DKIMError::KeyUnavailable(format!("failed to decode public key: {}", err))
        })?;
    match key_type {
        KeyType::Rsa => Ok(DkimPublicKey::Rsa(
            PKey::from_rsa(
                Rsa::public_key_from_der(&bytes)
                    .or_else(|_| Rsa::public_key_from_der_pkcs1(&bytes))
                    .map_err(|err| {
                        DKIMError::KeyUnavailable(format!("failed to parse public key: {}", err))
                    })?,
            )
            .map_err(|err| {
                DKIMError::KeyUnavailable(format!("failed to parse public key: {}", err))
            })?,
        )),
        KeyType::Ed25519 => {
            let mut key_bytes = [0u8; ed25519_dalek::PUBLIC_KEY_LENGTH];
            if bytes.len() != key_bytes.len() {
                return Err(DKIMError::KeyUnavailable(format!(
                    "ed25519 public keys should be {} bytes in length, have: {}",
                    ed25519_dalek::PUBLIC_KEY_LENGTH,
                    bytes.len()
                )));
            }

            key_bytes.copy_from_slice(&bytes);

            Ok(DkimPublicKey::Ed25519(
                ed25519_dalek::VerifyingKey::from_bytes(&key_bytes).map_err(|err| {
                    DKIMError::KeyUnavailable(format!("failed to parse public key: {}", err))
                })?,
            ))
        }
    }
}

// https://datatracker.ietf.org/doc/html/rfc6376#section-6.1.2
pub(crate) async fn retrieve_public_keys(
    resolver: &dyn Resolver,
    domain: &str,
    subdomain: &str,
) -> Result<Vec<DkimPublicKey>, DKIMError> {
    let dns_name = format!("{}.{}.{}", subdomain, DNS_NAMESPACE, domain);
    let answer = resolver.resolve_txt(&dns_name).await?;
    if answer.records.is_empty() {
        return Err(DKIMError::KeyUnavailable(format!(
            "failed to resolve {dns_name}"
        )));
    }

    // Return multiple keys for when verifiying the signatures. During key
    // rotation they are often multiple keys to consider.
    let txt = answer.as_txt();
    let mut errors = vec![];
    let mut keys = vec![];
    for record in txt {
        match parse_single_key(&record) {
            Ok(key) => {
                keys.push(key);
            }
            Err(err) => {
                errors.push(err);
            }
        }
    }

    if !keys.is_empty() {
        Ok(keys)
    } else if errors.len() == 1 {
        Err(errors.pop().unwrap())
    } else {
        let mut reasons = vec![];
        for err in errors {
            reasons.push(format!("{err:?}"));
        }
        Err(DKIMError::KeyUnavailable(format!(
            "Error(s) parsing DKIM records: {}",
            reasons.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dns_resolver::TestResolver;

    #[tokio::test]
    async fn test_retrieve_public_key() {
        let resolver = TestResolver::default()
            .with_txt(
                "dkim._domainkey.cloudflare.com",
                "v=DKIM1; k=rsa; p=MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA6gmVDBSBJ0l1/33uAF0gwIsrjQV6nnYjL9DMX6+ez4NNJ2um0InYy128Rd+OlIhmdSld6g3tj3O6R+BwsYsQgU8RWE8VJaRybvPw2P3Asgms4uPrFWHSFiWMPH0P9i/oPwnUO9jZKHiz4+MzFC3bG8BacX7YIxCuWnDU8XNmNsRaLmrv9CHX4/3GHyoHSmDA1ETtyz9JHRCOC8ho8C7b4f2Auwedlau9Lid9LGBhozhgRFhrFwFMe93y34MO1clPbY6HwxpudKWBkMQCTlmXVRnkKxHlJ+fYCyC2jjpCIbGWj2oLxBtFOASWMESR4biW0ph2bsZXslcUSPMTVTkFxQIDAQAB".to_owned(),
            );

        retrieve_public_keys(&resolver, "cloudflare.com", "dkim")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_retrieve_public_key_incompatible_version() {
        let resolver = TestResolver::default().with_txt(
            "dkim._domainkey.cloudflare.com",
            "v=DKIM6; p=key".to_owned(),
        );

        let key = retrieve_public_keys(&resolver, "cloudflare.com", "dkim")
            .await
            .unwrap_err();
        assert_eq!(key, DKIMError::KeyIncompatibleVersion);
    }

    #[tokio::test]
    async fn test_retrieve_public_key_inappropriate_key_algorithm() {
        let resolver = TestResolver::default().with_txt(
            "dkim._domainkey.cloudflare.com",
            "v=DKIM1; p=key; k=foo".to_owned(),
        );

        let key = retrieve_public_keys(&resolver, "cloudflare.com", "dkim")
            .await
            .unwrap_err();
        assert_eq!(key, DKIMError::InappropriateKeyAlgorithm);
    }
}
