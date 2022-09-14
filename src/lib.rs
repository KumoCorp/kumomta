// Implementation of DKIM: https://datatracker.ietf.org/doc/html/rfc6376

use indexmap::map::IndexMap;
use rsa::PublicKey;
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
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

#[derive(Debug)]
pub(crate) enum DkimPublicKey {
    Rsa(RsaPublicKey),
    Ed25519(ed25519_dalek::PublicKey),
}

#[derive(Debug)]
pub enum DkimPrivateKey {
    Rsa(RsaPrivateKey),
    Ed25519(ed25519_dalek::Keypair),
}

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
fn verify_signature(
    hash_algo: hash::HashAlgo,
    header_hash: Vec<u8>,
    signature: Vec<u8>,
    public_key: DkimPublicKey,
) -> Result<bool, DKIMError> {
    Ok(match public_key {
        DkimPublicKey::Rsa(public_key) => public_key
            .verify(
                rsa::PaddingScheme::PKCS1v15Sign {
                    hash: Some(match hash_algo {
                        hash::HashAlgo::RsaSha1 => rsa::hash::Hash::SHA1,
                        hash::HashAlgo::RsaSha256 => rsa::hash::Hash::SHA2_256,
                        hash => {
                            return Err(DKIMError::UnsupportedHashAlgorithm(format!("{:?}", hash)))
                        }
                    }),
                },
                &header_hash,
                &signature,
            )
            .is_ok(),
        DkimPublicKey::Ed25519(public_key) => public_key
            .verify_strict(
                &header_hash,
                &ed25519_dalek::Signature::from_bytes(&signature)
                    .map_err(|err| DKIMError::SignatureSyntaxError(err.to_string()))?,
            )
            .is_ok(),
    })
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
        let value = String::from_utf8_lossy(h.get_value_raw());
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
        if signing_domain.to_lowercase() != from_domain.to_lowercase() {
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
    use crate::dns::Lookup;

    use super::*;

    struct MockResolver {}

    impl Lookup for MockResolver {
        fn lookup_txt<'a>(
            &'a self,
            name: &'a str,
        ) -> futures::future::BoxFuture<'a, Result<Vec<String>, DKIMError>> {
            match name {
                "brisbane._domainkey.football.example.com" => {
                    Box::pin(futures::future::ready(Ok(vec![
                        "v=DKIM1; k=ed25519; p=11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo="
                            .to_string(),
                    ])))
                }
                "newengland._domainkey.example.com" => Box::pin(futures::future::ready(Ok(vec![
                    "v=DKIM1; p=MIGJAoGBALVI635dLK4cJJAH3Lx6upo3X/Lm1tQz3mezcWTA3BUBnyIsdnRf57aD5BtNmhPrYYDlWlzw3UgnKisIxktkk5+iMQMlFtAS10JB8L3YadXNJY+JBcbeSi5TgJe4WFzNgW95FWDAuSTRXSWZfA/8xjflbTLDx0euFZOM7C4T0GwLAgMBAAE=".to_string(),
                ]))),
                _ => {
                    println!("asked to resolve: {}", name);
                    todo!()
                }
            }
        }
    }

    impl MockResolver {
        fn new() -> Self {
            MockResolver {}
        }
    }

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

    #[tokio::test]
    async fn test_validate_email_header_ed25519() {
        let raw_email = r#"DKIM-Signature: v=1; a=ed25519-sha256; c=relaxed/relaxed;
 d=football.example.com; i=@football.example.com;
 q=dns/txt; s=brisbane; t=1528637909; h=from : to :
 subject : date : message-id : from : subject : date;
 bh=2jUSOH9NhtVGCQWNr9BrIAPreKQjO6Sn7XIkfJVOzv8=;
 b=/gCrinpcQOoIfuHNQIbq4pgh9kyIK3AQUdt9OdqQehSwhEIug4D11Bus
 Fa3bT3FY5OsU7ZbnKELq+eXdp1Q1Dw==
DKIM-Signature: v=1; a=rsa-sha256; c=relaxed/relaxed;
 d=football.example.com; i=@football.example.com;
 q=dns/txt; s=test; t=1528637909; h=from : to : subject :
 date : message-id : from : subject : date;
 bh=2jUSOH9NhtVGCQWNr9BrIAPreKQjO6Sn7XIkfJVOzv8=;
 b=F45dVWDfMbQDGHJFlXUNB2HKfbCeLRyhDXgFpEL8GwpsRe0IeIixNTe3
 DhCVlUrSjV4BwcVcOF6+FF3Zo9Rpo1tFOeS9mPYQTnGdaSGsgeefOsk2Jz
 dA+L10TeYt9BgDfQNZtKdN1WO//KgIqXP7OdEFE4LjFYNcUxZQ4FADY+8=
From: Joe SixPack <joe@football.example.com>
To: Suzie Q <suzie@shopping.example.net>
Subject: Is dinner ready?
Date: Fri, 11 Jul 2003 21:00:37 -0700 (PDT)
Message-ID: <20030712040037.46341.5F8J@football.example.com>

Hi.

We lost the game.  Are you hungry yet?

Joe."#
            .replace("\n", "\r\n");

        let email = mailparse::parse_mail(raw_email.as_bytes()).unwrap();
        let h = email
            .headers
            .get_all_headers(HEADER)
            .first()
            .unwrap()
            .get_value_raw();
        let raw_header_dkim = String::from_utf8_lossy(h);

        let resolver: Arc<dyn Lookup> = Arc::new(MockResolver::new());

        let dkim_verify_result = verify_email_header(
            &slog::Logger::root(slog::Discard, slog::o!()),
            Arc::clone(&resolver),
            &validate_header(&raw_header_dkim).unwrap(),
            &email,
        )
        .await;

        assert!(dkim_verify_result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_email_header_rsa() {
        // unfortunately the original RFC spec had a typo, and the mail content differs
        // between algorithms
        // https://www.rfc-editor.org/errata_search.php?rfc=6376&rec_status=0
        let raw_email =
            r#"DKIM-Signature: a=rsa-sha256; bh=2jUSOH9NhtVGCQWNr9BrIAPreKQjO6Sn7XIkfJVOzv8=;
 c=simple/simple; d=example.com;
 h=Received:From:To:Subject:Date:Message-ID; i=joe@football.example.com;
 s=newengland; t=1615825284; v=1;
 b=Xh4Ujb2wv5x54gXtulCiy4C0e+plRm6pZ4owF+kICpYzs/8WkTVIDBrzhJP0DAYCpnL62T0G
 k+0OH8pi/yqETVjKtKk+peMnNvKkut0GeWZMTze0bfq3/JUK3Ln3jTzzpXxrgVnvBxeY9EZIL4g
 s4wwFRRKz/1bksZGSjD8uuSU=
Received: from client1.football.example.com  [192.0.2.1]
      by submitserver.example.com with SUBMISSION;
      Fri, 11 Jul 2003 21:01:54 -0700 (PDT)
From: Joe SixPack <joe@football.example.com>
To: Suzie Q <suzie@shopping.example.net>
Subject: Is dinner ready?
Date: Fri, 11 Jul 2003 21:00:37 -0700 (PDT)
Message-ID: <20030712040037.46341.5F8J@football.example.com>

Hi.

We lost the game. Are you hungry yet?

Joe.
"#
            .replace("\n", "\r\n");
        let email = mailparse::parse_mail(raw_email.as_bytes()).unwrap();
        let h = email
            .headers
            .get_all_headers(HEADER)
            .first()
            .unwrap()
            .get_value_raw();
        let raw_header_rsa = String::from_utf8_lossy(h);

        let resolver: Arc<dyn Lookup> = Arc::new(MockResolver::new());

        let dkim_verify_result = verify_email_header(
            &slog::Logger::root(slog::Discard, slog::o!()),
            Arc::clone(&resolver),
            &validate_header(&raw_header_rsa).unwrap(),
            &email,
        )
        .await;

        assert!(dkim_verify_result.is_ok());
    }
}
