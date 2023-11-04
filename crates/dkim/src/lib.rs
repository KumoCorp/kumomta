// Implementation of DKIM: https://datatracker.ietf.org/doc/html/rfc6376

use crate::errors::Status;
use crate::hash::HeaderList;
use base64::engine::general_purpose;
use base64::Engine;
use ed25519_dalek::SigningKey;
use hickory_resolver::TokioAsyncResolver;
use mailparsing::AuthenticationResult;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
use sha1::Sha1;
use sha2::Sha256;
use std::collections::BTreeMap;

#[macro_use]
extern crate quick_error;

pub mod canonicalization;
pub mod dns;
mod errors;
mod hash;
mod header;
mod parsed_email;
mod parser;
mod public_key;
#[cfg(test)]
mod roundtrip_test;
mod sign;

pub use errors::DKIMError;
use header::{DKIMHeader, HEADER};
pub use parsed_email::ParsedEmail;
pub use parser::{tag_list as parse_tag_list, Tag};
pub use sign::{Signer, SignerBuilder};

const DNS_NAMESPACE: &str = "_domainkey";

#[derive(Debug)]
pub(crate) enum DkimPublicKey {
    Rsa(RsaPublicKey),
    Ed25519(ed25519_dalek::VerifyingKey),
}

#[derive(Debug)]
pub enum DkimPrivateKey {
    Rsa(RsaPrivateKey),
    Ed25519(SigningKey),
    #[cfg(feature = "openssl")]
    OpenSSLRsa(openssl::rsa::Rsa<openssl::pkey::Private>),
}

impl DkimPrivateKey {
    /// Parse RSA key data into a DkimPrivateKey
    pub fn rsa_key(data: &[u8]) -> Result<Self, DKIMError> {
        let mut errors = vec![];

        #[cfg(feature = "openssl")]
        {
            use openssl::rsa::Rsa;

            match Rsa::private_key_from_pem(data) {
                Ok(key) => return Ok(Self::OpenSSLRsa(key)),
                Err(err) => errors.push(format!("openssl private_key_from_pem: {err:#}")),
            };
            match Rsa::private_key_from_der(data) {
                Ok(key) => return Ok(Self::OpenSSLRsa(key)),
                Err(err) => errors.push(format!("openssl private_key_from_der: {err:#}")),
            };
        }
        match RsaPrivateKey::from_pkcs1_der(data) {
            Ok(key) => return Ok(Self::Rsa(key)),
            Err(err) => errors.push(format!("from_pkcs1_der: {err:#}")),
        }
        match RsaPrivateKey::from_pkcs8_der(data) {
            Ok(key) => return Ok(Self::Rsa(key)),
            Err(err) => errors.push(format!("from_pkcs8_der: {err:#}")),
        }

        match std::str::from_utf8(data) {
            Ok(s) => {
                match RsaPrivateKey::from_pkcs1_pem(s) {
                    Ok(key) => return Ok(Self::Rsa(key)),
                    Err(err) => errors.push(format!("from_pkcs1_pem: {err:#}")),
                }
                match RsaPrivateKey::from_pkcs8_pem(s) {
                    Ok(key) => return Ok(Self::Rsa(key)),
                    Err(err) => errors.push(format!("from_pkcs8_pem: {err:#}")),
                }
            }
            Err(err) => errors.push(format!("from_pkcs1_pem: data is not UTF-8: {err:#}")),
        }

        Err(DKIMError::PrivateKeyLoadError(errors.join(". ")))
    }

    /// Load RSA key data from a file and parse it into a DkimPrivateKey
    pub fn rsa_key_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, DKIMError> {
        let path = path.as_ref();
        let data = std::fs::read(path).map_err(|err| {
            DKIMError::PrivateKeyLoadError(format!(
                "rsa_key_file: failed to read file {path:?}: {err:#}"
            ))
        })?;
        Self::rsa_key(&data)
    }

    /// Parse PKCS8 encoded ed25519 key data into a DkimPrivateKey.
    /// Both DER and PEM are supported
    pub fn ed25519_key(data: &[u8]) -> Result<Self, DKIMError> {
        let mut errors = vec![];

        match SigningKey::from_pkcs8_der(data) {
            Ok(key) => return Ok(Self::Ed25519(key)),
            Err(err) => errors.push(format!("Ed25519 SigningKey::from_pkcs8_der: {err:#}")),
        }

        match std::str::from_utf8(data) {
            Ok(s) => match SigningKey::from_pkcs8_pem(s) {
                Ok(key) => return Ok(Self::Ed25519(key)),
                Err(err) => errors.push(format!("Ed25519 SigningKey::from_pkcs8_pem: {err:#}")),
            },
            Err(err) => errors.push(format!("ed25519_key: data is not UTF-8: {err:#}")),
        }

        Err(DKIMError::PrivateKeyLoadError(errors.join(". ")))
    }
}

// https://datatracker.ietf.org/doc/html/rfc6376#section-6.1.3 Step 4
fn verify_signature(
    hash_algo: hash::HashAlgo,
    header_hash: &[u8],
    signature: &[u8],
    public_key: DkimPublicKey,
) -> Result<bool, DKIMError> {
    Ok(match public_key {
        DkimPublicKey::Rsa(public_key) => public_key
            .verify(
                match hash_algo {
                    hash::HashAlgo::RsaSha1 => Pkcs1v15Sign::new::<Sha1>(),
                    hash::HashAlgo::RsaSha256 => Pkcs1v15Sign::new::<Sha256>(),
                    hash => return Err(DKIMError::UnsupportedHashAlgorithm(format!("{:?}", hash))),
                },
                header_hash,
                signature,
            )
            .is_ok(),
        DkimPublicKey::Ed25519(public_key) => {
            let mut sig_bytes = [0u8; ed25519_dalek::Signature::BYTE_SIZE];
            if signature.len() != sig_bytes.len() {
                return Err(DKIMError::SignatureSyntaxError(format!(
                    "ed25519 signatures should be {} bytes in length, have: {}",
                    ed25519_dalek::Signature::BYTE_SIZE,
                    signature.len()
                )));
            }
            sig_bytes.copy_from_slice(signature);

            public_key
                .verify_strict(
                    header_hash,
                    &ed25519_dalek::Signature::from_bytes(&sig_bytes),
                )
                .is_ok()
        }
    })
}

async fn verify_email_header<'a>(
    resolver: &dyn dns::Lookup,
    dkim_header: &'a DKIMHeader,
    email: &'a ParsedEmail<'a>,
) -> Result<(), DKIMError> {
    let public_key = public_key::retrieve_public_key(
        resolver,
        dkim_header.get_required_tag("d"),
        dkim_header.get_required_tag("s"),
    )
    .await?;

    let (header_canonicalization_type, body_canonicalization_type) =
        parser::parse_canonicalization(dkim_header.get_tag("c"))?;
    let hash_algo = parser::parse_hash_algo(&dkim_header.get_required_tag("a"))?;
    let computed_body_hash = hash::compute_body_hash(
        body_canonicalization_type,
        dkim_header.parse_tag("l")?,
        hash_algo,
        email,
    )?;

    let header_list: Vec<String> = dkim_header
        .get_required_tag("h")
        .split(':')
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();

    let computed_headers_hash = hash::compute_headers_hash(
        header_canonicalization_type,
        &HeaderList::new(header_list),
        hash_algo,
        dkim_header,
        email,
    )?;
    tracing::debug!("body_hash {:?}", computed_body_hash);

    let header_body_hash = dkim_header.get_required_tag("bh");
    if header_body_hash != computed_body_hash {
        return Err(DKIMError::BodyHashDidNotVerify);
    }

    let signature = general_purpose::STANDARD
        .decode(dkim_header.get_required_tag("b"))
        .map_err(|err| {
            DKIMError::SignatureSyntaxError(format!("failed to decode signature: {}", err))
        })?;
    if !verify_signature(hash_algo, &computed_headers_hash, &signature, public_key)? {
        return Err(DKIMError::SignatureDidNotVerify);
    }

    Ok(())
}

/// Run the DKIM verification on the email providing an existing resolver
pub async fn verify_email_with_resolver<'a>(
    from_domain: &str,
    email: &'a ParsedEmail<'a>,
    resolver: &dyn dns::Lookup,
) -> Result<Vec<AuthenticationResult>, DKIMError> {
    let mut results = vec![];

    let mut dkim_headers = vec![];

    for h in email.get_headers().iter_named(HEADER) {
        if results.len() > 10 {
            // Limit DoS impact if a malicious message is filled
            // with signatures
            break;
        }

        let value = h.get_raw_value();
        match DKIMHeader::parse(&value) {
            Ok(v) => {
                dkim_headers.push(v);
            }
            Err(err) => {
                results.push(AuthenticationResult {
                    method: "dkim".to_string(),
                    method_version: None,
                    result: "permerror".to_string(),
                    reason: Some(format!("{err}")),
                    props: BTreeMap::new(),
                });
            }
        }
    }

    /// <https://datatracker.ietf.org/doc/html/rfc6008>
    /// The value associated with this item in the header field MUST be
    /// at least the first eight characters of the digital signature
    /// (the "b=" tag from a DKIM-Signature) for which a result is being
    /// relayed, and MUST be long enough to be unique among the results being
    /// reported.
    fn compute_header_b(b_tag: &str, headers: &[DKIMHeader]) -> String {
        let mut len = 8;

        'bigger: while len < b_tag.len() {
            for h in headers {
                let candidate = h.get_required_tag("b");
                if candidate == b_tag {
                    continue;
                }
                if b_tag[0..len] == candidate[0..len] {
                    len += 2;
                    continue 'bigger;
                }
            }
            return b_tag[0..len].to_string();
        }
        b_tag.to_string()
    }

    for dkim_header in &dkim_headers {
        let signing_domain = dkim_header.get_required_tag("d");
        let mut props = BTreeMap::new();

        props.insert("header.d".to_string(), signing_domain.to_string());
        props.insert("header.i".to_string(), format!("@{signing_domain}"));
        props.insert(
            "header.a".to_string(),
            dkim_header.get_required_tag("a").to_string(),
        );
        props.insert(
            "header.s".to_string(),
            dkim_header.get_required_tag("s").to_string(),
        );

        let b_tag = compute_header_b(dkim_header.get_required_tag("b"), &dkim_headers);
        props.insert("header.b".to_string(), b_tag);

        let mut reason = None;
        let result = match verify_email_header(resolver, &dkim_header, email).await {
            Ok(()) => {
                if signing_domain.eq_ignore_ascii_case(from_domain) {
                    "pass"
                } else {
                    let why = "mail-from-mismatch-signing-domain".to_string();
                    reason.replace(why.clone());
                    props.insert("policy.dkim-rules".to_string(), why);
                    "policy"
                }
            }
            Err(err) => {
                reason.replace(format!("{err}"));
                match err.status() {
                    Status::Tempfail => "temperror",
                    Status::Permfail => "permerror",
                }
            }
        };

        results.push(AuthenticationResult {
            method: "dkim".to_string(),
            method_version: None,
            result: result.to_string(),
            reason,
            props,
        });
    }

    Ok(results)
}

/// Run the DKIM verification on the email
pub async fn verify_email<'a>(
    from_domain: &str,
    email: &'a ParsedEmail<'a>,
) -> Result<Vec<AuthenticationResult>, DKIMError> {
    let resolver = TokioAsyncResolver::tokio_from_system_conf().map_err(|err| {
        DKIMError::UnknownInternalError(format!("failed to create DNS resolver: {}", err))
    })?;

    verify_email_with_resolver(from_domain, email, &resolver).await
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
        DKIMHeader::parse(header).unwrap();
    }

    #[test]
    fn test_validate_header_missing_tag() {
        let header = "v=1; a=rsa-sha256; bh=a; b=b";
        assert_eq!(
            DKIMHeader::parse(header).unwrap_err(),
            DKIMError::SignatureMissingRequiredTag("d")
        );
    }

    #[test]
    fn test_validate_header_domain_mismatch() {
        let header = r#"v=1; a=rsa-sha256; d=example.net; s=brisbane; i=foo@hein.com; h=headers; bh=hash; b=hash
        "#;
        assert_eq!(
            DKIMHeader::parse(header).unwrap_err(),
            DKIMError::DomainMismatch
        );
    }

    #[test]
    fn test_validate_header_incompatible_version() {
        let header = r#"v=3; a=rsa-sha256; d=example.net; s=brisbane; i=foo@example.net; h=headers; bh=hash; b=hash
        "#;
        assert_eq!(
            DKIMHeader::parse(header).unwrap_err(),
            DKIMError::IncompatibleVersion
        );
    }

    #[test]
    fn test_validate_header_missing_from_in_headers_signature() {
        let header = r#"v=1; a=rsa-sha256; d=example.net; s=brisbane; i=foo@example.net; h=Subject:A:B; bh=hash; b=hash
        "#;
        assert_eq!(
            DKIMHeader::parse(header).unwrap_err(),
            DKIMError::FromFieldNotSigned
        );
    }

    #[test]
    fn test_validate_header_expired_in_drift() {
        let mut now = chrono::Utc::now().naive_utc();
        now -= chrono::Duration::seconds(1);

        let header = format!("v=1; a=rsa-sha256; d=example.net; s=brisbane; i=foo@example.net; h=From:B; bh=hash; b=hash; x={}", now.timestamp());

        assert!(DKIMHeader::parse(&header).is_ok());
    }

    #[test]
    fn test_validate_header_expired() {
        let mut now = chrono::Utc::now().naive_utc();
        now -= chrono::Duration::hours(3);

        let header = format!("v=1; a=rsa-sha256; d=example.net; s=brisbane; i=foo@example.net; h=From:B; bh=hash; b=hash; x={}", now.timestamp());

        assert_eq!(
            DKIMHeader::parse(&header).unwrap_err(),
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
            .replace('\n', "\r\n");

        let email = ParsedEmail::parse(raw_email).unwrap();
        let raw_header_dkim = email
            .get_headers()
            .iter_named(HEADER)
            .next()
            .unwrap()
            .get_raw_value();

        let resolver = MockResolver::new();

        verify_email_header(
            &resolver,
            &DKIMHeader::parse(raw_header_dkim).unwrap(),
            &email,
        )
        .await
        .unwrap();
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
            .replace('\n', "\r\n");
        let email = ParsedEmail::parse(raw_email).unwrap();
        let raw_header_rsa = email
            .get_headers()
            .iter_named(HEADER)
            .next()
            .unwrap()
            .get_raw_value();

        let resolver = MockResolver::new();

        verify_email_header(
            &resolver,
            &DKIMHeader::parse(raw_header_rsa).unwrap(),
            &email,
        )
        .await
        .unwrap();
    }
}
