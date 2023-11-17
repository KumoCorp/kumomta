use crate::header::DKIMHeaderBuilder;
use crate::{canonicalization, hash, DKIMError, DkimPrivateKey, HeaderList, ParsedEmail, HEADER};
use base64::engine::general_purpose;
use base64::Engine;
use ed25519_dalek::Signer as _;
use rsa::Pkcs1v15Sign;
use sha1::Sha1;
use sha2::Sha256;

/// Builder for the Signer
pub struct SignerBuilder {
    signed_headers: Option<Vec<String>>,
    private_key: Option<DkimPrivateKey>,
    selector: Option<String>,
    signing_domain: Option<String>,
    time: Option<chrono::DateTime<chrono::offset::Utc>>,
    header_canonicalization: canonicalization::Type,
    body_canonicalization: canonicalization::Type,
    expiry: Option<chrono::Duration>,
}

impl SignerBuilder {
    /// New builder
    pub fn new() -> Self {
        Self {
            signed_headers: None,
            private_key: None,
            selector: None,
            signing_domain: None,
            expiry: None,
            time: None,

            header_canonicalization: canonicalization::Type::Simple,
            body_canonicalization: canonicalization::Type::Simple,
        }
    }

    /// Specify headers to be used in the DKIM signature
    /// The From: header is required.
    pub fn with_signed_headers(
        mut self,
        headers: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, DKIMError> {
        let headers: Vec<String> = headers.into_iter().map(Into::into).collect();

        if !headers.iter().any(|h| h.eq_ignore_ascii_case("from")) {
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
    pub fn with_selector(mut self, value: impl Into<String>) -> Self {
        self.selector = Some(value.into());
        self
    }

    /// Specify for which domain the email should be signed for
    pub fn with_signing_domain(mut self, value: impl Into<String>) -> Self {
        self.signing_domain = Some(value.into());
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
    /// Must be provided: signed_headers, private_key, selector and
    /// signing_domain.
    pub fn build(self) -> Result<Signer, DKIMError> {
        use DKIMError::BuilderError;

        let private_key = self
            .private_key
            .ok_or(BuilderError("missing required private key"))?;
        let hash_algo = match private_key {
            DkimPrivateKey::Rsa(_) => hash::HashAlgo::RsaSha256,
            #[cfg(feature = "openssl")]
            DkimPrivateKey::OpenSSLRsa(_) => hash::HashAlgo::RsaSha256,
            DkimPrivateKey::Ed25519(_) => hash::HashAlgo::Ed25519Sha256,
        };

        Ok(Signer {
            signed_headers: HeaderList::new(
                self.signed_headers
                    .ok_or(BuilderError("missing required signed headers"))?,
            ),
            private_key,
            selector: self
                .selector
                .ok_or(BuilderError("missing required selector"))?,
            signing_domain: self
                .signing_domain
                .ok_or(BuilderError("missing required signing domain"))?,
            header_canonicalization: self.header_canonicalization,
            body_canonicalization: self.body_canonicalization,
            expiry: self.expiry,
            hash_algo,
            time: self.time,
        })
    }
}

impl Default for SignerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Signer {
    signed_headers: HeaderList,
    private_key: DkimPrivateKey,
    selector: String,
    signing_domain: String,
    header_canonicalization: canonicalization::Type,
    body_canonicalization: canonicalization::Type,
    expiry: Option<chrono::Duration>,
    hash_algo: hash::HashAlgo,
    time: Option<chrono::DateTime<chrono::offset::Utc>>,
}

/// DKIM signer. Use the [SignerBuilder] to build an instance.
impl Signer {
    /// Sign a message
    /// As specified in <https://datatracker.ietf.org/doc/html/rfc6376#section-5>
    pub fn sign<'b>(&self, email: &'b ParsedEmail<'b>) -> Result<String, DKIMError> {
        let body_hash = self.compute_body_hash(email)?;
        let dkim_header_builder = self.dkim_header_builder(&body_hash)?;

        let header_hash = self.compute_header_hash(email, dkim_header_builder.clone())?;

        let signature = match &self.private_key {
            DkimPrivateKey::Rsa(private_key) => private_key
                .sign(
                    match &self.hash_algo {
                        hash::HashAlgo::RsaSha1 => Pkcs1v15Sign::new::<Sha1>(),
                        hash::HashAlgo::RsaSha256 => Pkcs1v15Sign::new::<Sha256>(),
                        hash => {
                            return Err(DKIMError::UnsupportedHashAlgorithm(format!("{:?}", hash)))
                        }
                    },
                    &header_hash,
                )
                .map_err(|err| DKIMError::FailedToSign(err.to_string()))?,
            DkimPrivateKey::Ed25519(signing_key) => {
                signing_key.sign(&header_hash).to_bytes().into()
            }
            #[cfg(feature = "openssl")]
            DkimPrivateKey::OpenSSLRsa(private_key) => {
                use foreign_types::ForeignType;

                let mut siglen = private_key.size();
                let mut sigbuf = vec![0u8; siglen as usize];

                // We need to grub around a bit to call into RSA_sign:
                // The higher level wrappers available in the openssl
                // crate only include EVP_DigestSign which doesn't
                // accept a pre-calculated digest like we have here.

                let status = unsafe {
                    openssl_sys::RSA_sign(
                        match self.hash_algo {
                            hash::HashAlgo::RsaSha1 => openssl_sys::NID_sha1,
                            hash::HashAlgo::RsaSha256 => openssl_sys::NID_sha256,
                            hash => {
                                return Err(DKIMError::UnsupportedHashAlgorithm(format!(
                                    "{:?}",
                                    hash
                                )))
                            }
                        },
                        header_hash.as_ptr(),
                        header_hash.len() as _,
                        // unsafety: sigbuf must be >= siglen in size
                        sigbuf.as_mut_ptr(),
                        &mut siglen,
                        private_key.as_ptr(),
                    )
                };

                if status != 1 || siglen == 0 {
                    return Err(DKIMError::FailedToSign(format!(
                        "RSA_sign failed status={status} siglen={siglen} {:?}",
                        openssl::error::Error::get()
                    )));
                }

                sigbuf.truncate(siglen as usize);
                sigbuf
            }
        };

        // add the signature into the DKIM header and generate the header
        let dkim_header = dkim_header_builder
            .add_tag("b", &general_purpose::STANDARD.encode(signature))
            .build();

        Ok(format!("{}: {}", HEADER, dkim_header.raw_bytes))
    }

    fn dkim_header_builder(&self, body_hash: &str) -> Result<DKIMHeaderBuilder, DKIMError> {
        let now = chrono::offset::Utc::now();

        let mut builder = DKIMHeaderBuilder::new()
            .add_tag("v", "1")
            .add_tag("a", self.hash_algo.algo_name())
            .add_tag("d", &self.signing_domain)
            .add_tag("s", &self.selector)
            .add_tag(
                "c",
                &format!(
                    "{}/{}",
                    self.header_canonicalization.canon_name(),
                    self.body_canonicalization.canon_name()
                ),
            )
            .add_tag("bh", body_hash)
            .set_signed_headers(&self.signed_headers);
        if let Some(expiry) = self.expiry {
            builder = builder.set_expiry(expiry)?;
        }
        if let Some(time) = self.time {
            builder = builder.set_time(time);
        } else {
            builder = builder.set_time(now);
        }

        Ok(builder)
    }

    fn compute_body_hash<'b>(&self, email: &'b ParsedEmail<'b>) -> Result<String, DKIMError> {
        let length = None;
        let canonicalization = self.body_canonicalization;
        hash::compute_body_hash(canonicalization, length, self.hash_algo, email)
    }

    fn compute_header_hash<'b>(
        &self,
        email: &'b ParsedEmail<'b>,
        dkim_header_builder: DKIMHeaderBuilder,
    ) -> Result<Vec<u8>, DKIMError> {
        let canonicalization = self.header_canonicalization;

        // For signing the DKIM-Signature header the signature needs to be null
        let dkim_header = dkim_header_builder.add_tag("b", "").build();

        hash::compute_headers_hash(
            canonicalization,
            &self.signed_headers,
            self.hash_algo,
            &dkim_header,
            email,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs;

    #[test]
    fn test_sign_rsa() {
        let raw_email = r#"Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
        .replace("\n", "\r\n");
        let email = ParsedEmail::parse(raw_email).unwrap();

        let private_key = DkimPrivateKey::rsa_key_file("./test/keys/2022.private").unwrap();
        let time = chrono::Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 1).unwrap();

        let signer = SignerBuilder::new()
            .with_signed_headers(["From", "Subject"])
            .unwrap()
            .with_private_key(private_key)
            .with_selector("s20")
            .with_signing_domain("example.com")
            .with_time(time)
            .build()
            .unwrap();
        let header = signer.sign(&email).unwrap();

        k9::snapshot!(
            header,
            r#"
DKIM-Signature: v=1; a=rsa-sha256; d=example.com; s=s20; c=simple/simple;\r
\tbh=KXQwQpX2zFwgixPbV6Dd18ZMJU04lLeRnwqzUp8uGwI=;\r
\th=from:subject; t=1609459201;\r
\tb=jWvcCA6TzqyFbpitXBo2barOzu7ObOcPg5jqqdekMdHTxR2XoAGGtQ9NUDVqxJoifZvOIfElh\r
\tT7717zandgj4HSL0nldmfhLHECN43Ktk3dfpSid5KPZQJddQBVwrH6qUXPoAk9THhuZx8KP/PdM\r
\tedlRuNYixoMtZynSl8VfWOjMQohanxafYUtIG+p2DYCq82uzVOLy87mvQBk8IWooNk1rDTHkj5U\r
\t03xSRjPuEUZqkQKJzYcPV+L9TE3jX7HmuCzRpY9fn3G0xp/YhJFD7FuGr47vZLzMRaqqov5BTJw\r
\tTgKxK8IE0fuYkF7e1LUYbEzZqdtSLxgmzCuz+efLY38w==;
"#
        );
    }

    #[cfg(feature = "openssl")]
    #[test]
    fn test_sign_rsa_openssl() {
        let raw_email = r#"Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
        .replace("\n", "\r\n");
        let email = ParsedEmail::parse(raw_email).unwrap();

        let data = std::fs::read("./test/keys/2022.private").unwrap();
        let pkey = openssl::rsa::Rsa::private_key_from_pem(&data).unwrap();

        let time = chrono::Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 1).unwrap();

        let signer = SignerBuilder::new()
            .with_signed_headers([
                "From",
                "Subject",
                "List-Unsubscribe",
                "List-Unsubscribe-Post",
                "X-Lets-Make-This-Really-Long",
            ])
            .unwrap()
            .with_private_key(DkimPrivateKey::OpenSSLRsa(pkey))
            .with_selector("s20")
            .with_signing_domain("example.com")
            .with_time(time)
            .build()
            .unwrap();
        let header = signer.sign(&email).unwrap();

        k9::snapshot!(
            header,
            r#"
DKIM-Signature: v=1; a=rsa-sha256; d=example.com; s=s20; c=simple/simple;\r
\tbh=KXQwQpX2zFwgixPbV6Dd18ZMJU04lLeRnwqzUp8uGwI=;\r
\th=from:subject:list-unsubscribe:list-unsubscribe-post:\r
\t\tx-lets-make-this-really-long; t=1609459201;\r
\tb=kNEseaF1ozpjc3/BnUgXqRjl99TIOmxnIlXzQEGu9B3HkUmiZM3sY9jkoqo3x44DlxZv2sEsd\r
\todQQ8NivIvruQb7tkgRrnhB+54fVh7mfxiG3q1CB3fFkz13FPU85UkE/Y5HozEfjfSBiBDMnguv\r
\tZyh/M4SVbDAXxBeQWHVVggkUQoyRy7X9vdlK3vRWQq+mdFINEUITKSI6GAUJdtWDTUad3/DnOm5\r
\tykzWZkIcX7u+ng2jXC7wI+cko4+dLzdy9SIKaL1rEqdiF+IDRnR1yLDBZjQXUyzPkLYKzmrOAsb\r
\tF1E9z34xwGjT0F3+TKbcupxg8mHnn0QBU8PXCKb+NYbQ==;
"#
        );
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
            .replace('\n', "\r\n");
        let email = ParsedEmail::parse(raw_email).unwrap();

        let file_content = fs::read("./test/keys/ed.private").unwrap();
        let file_decoded = general_purpose::STANDARD.decode(file_content).unwrap();
        let mut key_bytes = [0u8; ed25519_dalek::SECRET_KEY_LENGTH];
        key_bytes.copy_from_slice(&file_decoded);
        let secret_key = ed25519_dalek::SigningKey::from_bytes(&key_bytes);

        let time = chrono::Utc
            .with_ymd_and_hms(2018, 6, 10, 13, 38, 29)
            .unwrap();

        let signer = SignerBuilder::new()
            .with_signed_headers([
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
            .with_private_key(DkimPrivateKey::Ed25519(secret_key))
            .with_body_canonicalization(canonicalization::Type::Relaxed)
            .with_header_canonicalization(canonicalization::Type::Relaxed)
            .with_selector("brisbane")
            .with_signing_domain("football.example.com")
            .with_time(time)
            .build()
            .unwrap();
        let header = signer.sign(&email).unwrap();

        k9::snapshot!(
            header,
            r#"
DKIM-Signature: v=1; a=ed25519-sha256; d=football.example.com; s=brisbane;\r
\tc=relaxed/relaxed; bh=2jUSOH9NhtVGCQWNr9BrIAPreKQjO6Sn7XIkfJVOzv8=;\r
\th=from:to:subject:date:message-id:from:subject:date; t=1528637909;\r
\tb=wITr2H3sBuBfMsnUwlRTO7Oq/C/jd2vubDm50DrXtMFEBLRiz9GfrgCozcg764+gYqWXV3Snd\r
\t1ynYh8sJ5BXBg==;
"#
        );
    }
}
