// Implementation of DKIM: https://datatracker.ietf.org/doc/html/rfc6376

use indexmap::map::IndexMap;
use slog::debug;
use std::collections::HashSet;
use std::sync::Arc;
use trust_dns_resolver::TokioAsyncResolver;

use mailparse::MailHeaderMap;

#[macro_use]
extern crate quick_error;

mod bytes;
pub mod canonicalization;
pub mod dns;
mod errors;
mod hash;
mod header;
mod parser;
mod public_key;
mod result;
#[cfg(test)]
mod roundtrip_test;
mod sign;

pub use errors::DKIMError;
use header::{DKIMHeader, HEADER, REQUIRED_TAGS};
pub use parser::tag_list as parse_tag_list;
pub use parser::Tag;
pub use result::DKIMResult;
pub use sign::{Signer, SignerBuilder};

const SIGN_EXPIRATION_DRIFT_MINS: i64 = 15;
const DNS_NAMESPACE: &str = "_domainkey";

// https://datatracker.ietf.org/doc/html/rfc6376#section-6.1.1
fn validate_header<'a>(value: &'a str) -> Result<DKIMHeader, DKIMError> {
    let (_, tags) =
        parser::tag_list(value).map_err(|err| DKIMError::SignatureSyntaxError(err.to_string()))?;

    // Check presence of required tags
    {
        let mut tag_names: HashSet<String> = HashSet::new();
        for tag in &tags {
            tag_names.insert(tag.name.clone());
        }
        for required in REQUIRED_TAGS {
            if tag_names.get(*required).is_none() {
                return Err(DKIMError::SignatureMissingRequiredTag(required));
            }
        }
    }

    let mut tags_map = IndexMap::new();
    for tag in &tags {
        tags_map.insert(tag.name.clone(), tag.clone());
    }
    let header = DKIMHeader {
        tags: tags_map,
        raw_bytes: value.to_owned(),
    };
    // FIXME: we could get the keys instead of generating tag_names ourselves

    // Check version
    {
        let version = header.get_required_tag("v");
        if version != "1" {
            return Err(DKIMError::IncompatibleVersion);
        }
    }

    // Check that "d=" tag is the same as or a parent domain of the domain part
    // of the "i=" tag
    if let Some(user) = header.get_tag("i") {
        let signing_domain = header.get_required_tag("d");
        // TODO: naive check, should switch to parsing the domains/email
        if !user.ends_with(&signing_domain) {
            return Err(DKIMError::DomainMismatch);
        }
    }

    // Check that "h=" tag includes the From header
    {
        let value = header.get_required_tag("h");
        let headers = value.split(":");
        let headers: Vec<String> = headers.map(|h| h.to_lowercase()).collect();
        if !headers.contains(&"from".to_string()) {
            return Err(DKIMError::FromFieldNotSigned);
        }
    }

    if let Some(query_method) = header.get_tag("q") {
        if query_method != "dns/txt" {
            return Err(DKIMError::UnsupportedQueryMethod);
        }
    }

    // Check that "x=" tag isn't expired
    if let Some(expiration) = header.get_tag("x") {
        let mut expiration =
            chrono::NaiveDateTime::from_timestamp(expiration.parse::<i64>().unwrap_or_default(), 0);
        expiration += chrono::Duration::minutes(SIGN_EXPIRATION_DRIFT_MINS);
        let now = chrono::Utc::now().naive_utc();
        if now > expiration {
            return Err(DKIMError::SignatureExpired);
        }
    }

    Ok(header)
}

// https://datatracker.ietf.org/doc/html/rfc6376#section-6.1.3 Step 4
// TODO: implement verification with ed25519 keys
fn verify_signature(
    hash_algo: hash::HashAlgo,
    header_hash: Vec<u8>,
    signature: Vec<u8>,
    public_key: impl rsa::PublicKey,
) -> Result<bool, DKIMError> {
    Ok(public_key
        .verify(
            rsa::PaddingScheme::PKCS1v15Sign {
                hash: Some(match hash_algo {
                    hash::HashAlgo::RsaSha1 => rsa::hash::Hash::SHA1,
                    hash::HashAlgo::RsaSha256 => rsa::hash::Hash::SHA2_256,
                }),
            },
            &header_hash,
            &signature,
        )
        .is_ok())
}

async fn verify_email_header<'a>(
    logger: &'a slog::Logger,
    resolver: Arc<dyn dns::Lookup>,
    dkim_header: &'a DKIMHeader,
    email: &'a mailparse::ParsedMail<'a>,
) -> Result<(canonicalization::Type, canonicalization::Type), DKIMError> {
    let public_key = public_key::retrieve_public_key(
        logger,
        Arc::clone(&resolver),
        dkim_header.get_required_tag("d"),
        dkim_header.get_required_tag("s"),
        dkim_header.get_tag("k"),
    )
    .await?;

    let (header_canonicalization_type, body_canonicalization_type) =
        parser::parse_canonicalization(dkim_header.get_tag("c"))?;
    let hash_algo = parser::parse_hash_algo(&dkim_header.get_required_tag("a"))?;
    let computed_body_hash = hash::compute_body_hash(
        body_canonicalization_type.clone(),
        dkim_header.get_tag("l"),
        hash_algo.clone(),
        email,
    )?;
    let computed_headers_hash = hash::compute_headers_hash(
        logger,
        header_canonicalization_type.clone(),
        &dkim_header.get_required_tag("h"),
        hash_algo.clone(),
        &dkim_header,
        email,
    )?;
    debug!(logger, "body_hash {:?}", computed_body_hash);

    let header_body_hash = dkim_header.get_required_tag("bh").clone();
    if header_body_hash != computed_body_hash {
        return Err(DKIMError::BodyHashDidNotVerify);
    }

    let signature = base64::decode(dkim_header.get_required_tag("b")).map_err(|err| {
        DKIMError::SignatureSyntaxError(format!("failed to decode signature: {}", err))
    })?;
    if !verify_signature(hash_algo, computed_headers_hash, signature, public_key)? {
        return Err(DKIMError::SignatureDidNotVerify);
    }

    Ok((header_canonicalization_type, body_canonicalization_type))
}

/// Run the DKIM verification on the email providing an existing resolver
pub async fn verify_email_with_resolver<'a>(
    logger: &slog::Logger,
    from_domain: &str,
    email: &'a mailparse::ParsedMail<'a>,
    resolver: Arc<dyn dns::Lookup>,
) -> Result<DKIMResult, DKIMError> {
    let mut last_error = None;

    for h in email.headers.get_all_headers(HEADER) {
        let value = h.get_value();
        debug!(logger, "checking signature {:?}", value);

        let dkim_header = match validate_header(&value) {
            Ok(v) => v,
            Err(err) => {
                debug!(logger, "failed to verify: {}", err);
                last_error = Some(err);
                continue;
            }
        };

        // Select the signature corresponding to the email sender
        let signing_domain = dkim_header.get_required_tag("d");
        if signing_domain != from_domain {
            continue;
        }

        match verify_email_header(logger, Arc::clone(&resolver), &dkim_header, email).await {
            Ok((header_canonicalization_type, body_canonicalization_type)) => {
                return Ok(DKIMResult::pass(
                    signing_domain,
                    header_canonicalization_type,
                    body_canonicalization_type,
                ))
            }
            Err(err) => {
                debug!(logger, "failed to verify: {}", err);
                last_error = Some(err);
                continue;
            }
        }
    }

    if let Some(err) = last_error {
        Ok(DKIMResult::fail(err, from_domain.to_owned()))
    } else {
        Ok(DKIMResult::neutral(from_domain.to_owned()))
    }
}

/// Run the DKIM verification on the email
pub async fn verify_email<'a>(
    logger: &slog::Logger,
    from_domain: &str,
    email: &'a mailparse::ParsedMail<'a>,
) -> Result<DKIMResult, DKIMError> {
    let resolver = TokioAsyncResolver::tokio_from_system_conf().map_err(|err| {
        DKIMError::UnknownInternalError(format!("failed to create DNS resolver: {}", err))
    })?;
    let resolver = dns::from_tokio_resolver(resolver);

    verify_email_with_resolver(logger, from_domain, email, resolver).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_header() {
        let header = r#"v=1; a=rsa-sha256; d=example.net; s=brisbane;
c=relaxed/simple; q=dns/txt; i=foo@eng.example.net;
t=1117574938; x=9118006938; l=200;
h=from:to:subject:date:keywords:keywords;
z=From:foo@eng.example.net|To:joe@example.com|
Subject:demo=20run|Date:July=205,=202005=203:44:08=20PM=20-0700;
bh=MTIzNDU2Nzg5MDEyMzQ1Njc4OTAxMjM0NTY3ODkwMTI=;
b=dzdVyOfAKCdLXdJOc9G2q8LoXSlEniSbav+yuU4zGeeruD00lszZ
      VoG4ZHRNiYzR
        "#;
        validate_header(header).unwrap();
    }

    #[test]
    fn test_validate_header_missing_tag() {
        let header = "v=1; a=rsa-sha256; bh=a; b=b";
        assert_eq!(
            validate_header(header).unwrap_err(),
            DKIMError::SignatureMissingRequiredTag("d")
        );
    }

    #[test]
    fn test_validate_header_domain_mismatch() {
        let header = r#"v=1; a=rsa-sha256; d=example.net; s=brisbane; i=foo@hein.com; h=headers; bh=hash; b=hash
        "#;
        assert_eq!(
            validate_header(header).unwrap_err(),
            DKIMError::DomainMismatch
        );
    }

    #[test]
    fn test_validate_header_incompatible_version() {
        let header = r#"v=3; a=rsa-sha256; d=example.net; s=brisbane; i=foo@example.net; h=headers; bh=hash; b=hash
        "#;
        assert_eq!(
            validate_header(header).unwrap_err(),
            DKIMError::IncompatibleVersion
        );
    }

    #[test]
    fn test_validate_header_missing_from_in_headers_signature() {
        let header = r#"v=1; a=rsa-sha256; d=example.net; s=brisbane; i=foo@example.net; h=Subject:A:B; bh=hash; b=hash
        "#;
        assert_eq!(
            validate_header(header).unwrap_err(),
            DKIMError::FromFieldNotSigned
        );
    }

    #[test]
    fn test_validate_header_expired_in_drift() {
        let mut now = chrono::Utc::now().naive_utc();
        now -= chrono::Duration::seconds(1);

        let header = format!("v=1; a=rsa-sha256; d=example.net; s=brisbane; i=foo@example.net; h=From:B; bh=hash; b=hash; x={}", now.timestamp());

        assert!(validate_header(&header).is_ok());
    }

    #[test]
    fn test_validate_header_expired() {
        let mut now = chrono::Utc::now().naive_utc();
        now -= chrono::Duration::hours(3);

        let header = format!("v=1; a=rsa-sha256; d=example.net; s=brisbane; i=foo@example.net; h=From:B; bh=hash; b=hash; x={}", now.timestamp());

        assert_eq!(
            validate_header(&header).unwrap_err(),
            DKIMError::SignatureExpired
        );
    }
}
