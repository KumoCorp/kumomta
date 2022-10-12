use ed25519_dalek::ExpandedSecretKey;

use crate::header::DKIMHeaderBuilder;
use crate::{canonicalization, hash, DKIMError, DkimPrivateKey, HEADER};

/// Builder for the Signer
pub struct SignerBuilder<'a> {
    signed_headers: Option<&'a [&'a str]>,
    private_key: Option<DkimPrivateKey>,
    selector: Option<&'a str>,
    signing_domain: Option<&'a str>,
    time: Option<chrono::DateTime<chrono::offset::Utc>>,
    header_canonicalization: canonicalization::Type,
    body_canonicalization: canonicalization::Type,
    logger: Option<&'a slog::Logger>,
    expiry: Option<chrono::Duration>,
}

impl<'a> SignerBuilder<'a> {
    /// New builder
    pub fn new() -> Self {
        Self {
            signed_headers: None,
            private_key: None,
            selector: None,
            logger: None,
            signing_domain: None,
            expiry: None,
            time: None,

            header_canonicalization: canonicalization::Type::Simple,
            body_canonicalization: canonicalization::Type::Simple,
        }
    }

    /// Specify headers to be used in the DKIM signature
    /// The From: header is required.
    pub fn with_signed_headers(mut self, headers: &'a [&'a str]) -> Result<Self, DKIMError> {
        let from = headers.iter().find(|h| h.to_lowercase() == "from");
        if from.is_none() {
            return Err(DKIMError::BuilderError("missing From in signed headers"));
        }

        self.signed_headers = Some(headers);
        Ok(self)
    }

    /// Specify the private key used to sign the email
    pub fn with_private_key(mut self, key: DkimPrivateKey) -> Self {
        self.private_key = Some(key);
        self
    }

    /// Specify the private key used to sign the email
    pub fn with_selector(mut self, value: &'a str) -> Self {
        self.selector = Some(value);
        self
    }

    /// Specify for which domain the email should be signed for
    pub fn with_signing_domain(mut self, value: &'a str) -> Self {
        self.signing_domain = Some(value);
        self
    }

    /// Specify the header canonicalization
    pub fn with_header_canonicalization(mut self, value: canonicalization::Type) -> Self {
        self.header_canonicalization = value;
        self
    }

    /// Specify the body canonicalization
    pub fn with_body_canonicalization(mut self, value: canonicalization::Type) -> Self {
        self.body_canonicalization = value;
        self
    }

    /// Specify a logger
    pub fn with_logger(mut self, logger: &'a slog::Logger) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Specify current time. Mostly used for testing
    pub fn with_time(mut self, value: chrono::DateTime<chrono::offset::Utc>) -> Self {
        self.time = Some(value);
        self
    }

    /// Specify a expiry duration for the signature validity
    pub fn with_expiry(mut self, value: chrono::Duration) -> Self {
        self.expiry = Some(value);
        self
    }

    /// Build an instance of the Signer
    /// Must be provided: signed_headers, private_key, selector, logger and
    /// signing_domain.
    pub fn build(self) -> Result<Signer<'a>, DKIMError> {
        use DKIMError::BuilderError;

        let private_key = self
            .private_key
            .ok_or(BuilderError("missing required private key"))?;
        let hash_algo = match private_key {
            DkimPrivateKey::Rsa(_) => hash::HashAlgo::RsaSha256,
            DkimPrivateKey::Ed25519(_) => hash::HashAlgo::Ed25519Sha256,
        };

        Ok(Signer {
            signed_headers: self
                .signed_headers
                .ok_or(BuilderError("missing required signed headers"))?,
            private_key: private_key,
            selector: self
                .selector
                .ok_or(BuilderError("missing required selector"))?,
            logger: self.logger.ok_or(BuilderError("missing required logger"))?,
            signing_domain: self
                .signing_domain
                .ok_or(BuilderError("missing required logger"))?,
            header_canonicalization: self.header_canonicalization,
            body_canonicalization: self.body_canonicalization,
            expiry: self.expiry,
            hash_algo: hash_algo,
            time: self.time,
        })
    }
}

pub struct Signer<'a> {
    signed_headers: &'a [&'a str],
    private_key: DkimPrivateKey,
    selector: &'a str,
    signing_domain: &'a str,
    header_canonicalization: canonicalization::Type,
    body_canonicalization: canonicalization::Type,
    logger: &'a slog::Logger,
    expiry: Option<chrono::Duration>,
    hash_algo: hash::HashAlgo,
    time: Option<chrono::DateTime<chrono::offset::Utc>>,
}

/// DKIM signer. Use the [SignerBuilder] to build an instance.
impl<'a> Signer<'a> {
    /// Sign a message
    /// As specified in <https://datatracker.ietf.org/doc/html/rfc6376#section-5>
    pub fn sign<'b>(&self, email: &'b mailparse::ParsedMail<'b>) -> Result<String, DKIMError> {
        let body_hash = self.compute_body_hash(email)?;
        let dkim_header_builder = self.dkim_header_builder(&body_hash)?;

        let header_hash = self.compute_header_hash(email, dkim_header_builder.clone())?;

        let signature = match &self.private_key {
            DkimPrivateKey::Rsa(private_key) => private_key
                .sign(
                    rsa::PaddingScheme::PKCS1v15Sign {
                        hash: Some(match &self.hash_algo {
                            hash::HashAlgo::RsaSha1 => rsa::hash::Hash::SHA1,
                            hash::HashAlgo::RsaSha256 => rsa::hash::Hash::SHA2_256,
                            hash => {
                                return Err(DKIMError::UnsupportedHashAlgorithm(format!(
                                    "{:?}",
                                    hash
                                )))
                            }
                        }),
                    },
                    &header_hash,
                )
                .map_err(|err| DKIMError::FailedToSign(err.to_string()))?,
            DkimPrivateKey::Ed25519(keypair) => {
                let expanded: ExpandedSecretKey = (&keypair.secret).into();
                expanded
                    .sign(&header_hash, &keypair.public)
                    .to_bytes()
                    .into()
            }
        };

        // add the signature into the DKIM header and generate the header
        let dkim_header = dkim_header_builder
            .add_tag("b", &base64::encode(&signature))
            .build()?;

        Ok(format!("{}: {}", HEADER, dkim_header.raw_bytes))
    }

    fn dkim_header_builder(&self, body_hash: &str) -> Result<DKIMHeaderBuilder, DKIMError> {
        let now = chrono::offset::Utc::now();
        let hash_algo = match self.hash_algo {
            hash::HashAlgo::RsaSha1 => "rsa-sha1",
            hash::HashAlgo::RsaSha256 => "rsa-sha256",
            hash::HashAlgo::Ed25519Sha256 => "ed25519-sha256",
        };

        let mut builder = DKIMHeaderBuilder::new()
            .add_tag("v", "1")
            .add_tag("a", hash_algo)
            .add_tag("d", &self.signing_domain)
            .add_tag("s", self.selector)
            .add_tag(
                "c",
                &format!(
                    "{}/{}",
                    self.header_canonicalization.to_string(),
                    self.body_canonicalization.to_string()
                ),
            )
            .add_tag("bh", body_hash)
            .set_signed_headers(self.signed_headers);
        if let Some(expiry) = self.expiry {
            builder = builder.set_expiry(expiry)?;
        }
        if let Some(time) = self.time {
            builder = builder.set_time(time);
        } else {
            builder = builder.set_time(now.into());
        }

        Ok(builder)
    }

    fn compute_body_hash<'b>(
        &self,
        email: &'b mailparse::ParsedMail<'b>,
    ) -> Result<String, DKIMError> {
        let length = None;
        let canonicalization = self.body_canonicalization.clone();
        hash::compute_body_hash(canonicalization, length, self.hash_algo.clone(), email)
    }

    fn compute_header_hash<'b>(
        &self,
        email: &'b mailparse::ParsedMail<'b>,
        dkim_header_builder: DKIMHeaderBuilder,
    ) -> Result<Vec<u8>, DKIMError> {
        let canonicalization = self.header_canonicalization.clone();

        // For signing the DKIM-Signature header the signature needs to be null
        let dkim_header = dkim_header_builder.add_tag("b", "").build()?;
        let signed_headers = dkim_header.get_required_tag("h");

        hash::compute_headers_hash(
            self.logger,
            canonicalization,
            &signed_headers,
            self.hash_algo.clone(),
            &dkim_header,
            email,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use std::{fs, path::Path};

    fn test_logger() -> slog::Logger {
        slog::Logger::root(slog::Discard, slog::o!())
    }

    #[test]
    fn test_sign_rsa() {
        let email = mailparse::parse_mail(
            r#"Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
            .as_bytes(),
        )
        .unwrap();

        let private_key =
            rsa::RsaPrivateKey::read_pkcs1_pem_file(Path::new("./test/keys/2022.private")).unwrap();
        let logger = test_logger();
        let time = chrono::Utc.ymd(2021, 1, 1).and_hms_milli(0, 0, 1, 444);

        let signer = SignerBuilder::new()
            .with_signed_headers(&["From", "Subject"])
            .unwrap()
            .with_private_key(DkimPrivateKey::Rsa(private_key))
            .with_selector("s20")
            .with_logger(&logger)
            .with_signing_domain("example.com")
            .with_time(time)
            .build()
            .unwrap();
        let header = signer.sign(&email).unwrap();

        assert_eq!(header, "DKIM-Signature: v=1; a=rsa-sha256; d=example.com; s=s20; c=simple/simple; bh=frcCV1k9oG9oKj3dpUqdJg1PxRT2RSN/XKdLCPjaYaY=; h=from:subject; t=1609459201; b=ohfeeUk89mJI/nTb8cViCbOY11tYBkj0xecrpXVwPdkvLMYMZemydr01nUuruhrzaqxFcqgjdEB/alen4NygDo3Kj//GsEUksRO13Hi1aW5lfxLj7Ifux96CbKm3EEcI5rD9tXQ0LaW5nYUdqYdFVIgmU/qTtXRenMxesHhggknm1n6x7K4NsqBS+9leidXtKf8hTSCC7f4XMGFe2YQrCKHfYFBb/MTuzCHbF/CgZHKgMhBAYXMkuEwIGjh4xnR256AmJdxHN+JdrWYzkMdRiuDmYvlnUJdPWq0hD3fR1DxS5/YF6hNHMP9b1yM8eiUQVnqrbzR8C5KWJiM8JhaBcg==;")
    }

    #[test]
    fn test_sign_ed25519() {
        let raw_email = r#"From: Joe SixPack <joe@football.example.com>
To: Suzie Q <suzie@shopping.example.net>
Subject: Is dinner ready?
Date: Fri, 11 Jul 2003 21:00:37 -0700 (PDT)
Message-ID: <20030712040037.46341.5F8J@football.example.com>

Hi.

We lost the game.  Are you hungry yet?

Joe."#
            .replace("\n", "\r\n");
        let email = mailparse::parse_mail(raw_email.as_bytes()).unwrap();

        let file_content = fs::read("./test/keys/ed.private").unwrap();
        let file_decoded = base64::decode(file_content).unwrap();
        let secret_key = ed25519_dalek::SecretKey::from_bytes(&file_decoded).unwrap();

        let file_content = fs::read("./test/keys/ed.public").unwrap();
        let file_decoded = base64::decode(file_content).unwrap();
        let public_key = ed25519_dalek::PublicKey::from_bytes(&file_decoded).unwrap();

        let keypair = ed25519_dalek::Keypair {
            public: public_key,
            secret: secret_key,
        };

        let logger = test_logger();
        let time = chrono::Utc.ymd(2018, 6, 10).and_hms_milli(13, 38, 29, 444);

        let signer = SignerBuilder::new()
            .with_signed_headers(&[
                "From",
                "To",
                "Subject",
                "Date",
                "Message-ID",
                "From",
                "Subject",
                "Date",
            ])
            .unwrap()
            .with_private_key(DkimPrivateKey::Ed25519(keypair))
            .with_body_canonicalization(canonicalization::Type::Relaxed)
            .with_header_canonicalization(canonicalization::Type::Relaxed)
            .with_selector("brisbane")
            .with_logger(&logger)
            .with_signing_domain("football.example.com")
            .with_time(time)
            .build()
            .unwrap();
        let header = signer.sign(&email).unwrap();

        assert_eq!(header, "DKIM-Signature: v=1; a=ed25519-sha256; d=football.example.com; s=brisbane; c=relaxed/relaxed; bh=2jUSOH9NhtVGCQWNr9BrIAPreKQjO6Sn7XIkfJVOzv8=; h=from:to:subject:date:message-id:from:subject:date; t=1528637909; b=wITr2H3sBuBfMsnUwlRTO7Oq/C/jd2vubDm50DrXtMFEBLRiz9GfrgCozcg764+gYqWXV3Snd1ynYh8sJ5BXBg==;")
    }
}
