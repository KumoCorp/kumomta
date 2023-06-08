use anyhow::Context;
use cfdkim::DkimPrivateKey;
use config::get_or_create_sub_module;
use data_loader::KeySource;
use lruttl::LruCacheWithTtl;
use mail_auth::common::crypto::{Ed25519Key, HashAlgorithm, RsaKey, Sha256, SigningKey};
use mail_auth::common::headers::HeaderWriter;
use mail_auth::dkim::{Canonicalization, DkimSigner, Done, NeedDomain};
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, Value};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::RsaPrivateKey;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

lazy_static::lazy_static! {
    static ref SIGNER_CACHE: LruCacheWithTtl<SignerConfig, Arc<SignerInner>> = LruCacheWithTtl::new(1024);
}

#[derive(Deserialize, Hash, Eq, PartialEq, Copy, Clone)]
pub enum Canon {
    Relaxed,
    Simple,
}

impl Default for Canon {
    fn default() -> Self {
        Self::Relaxed
    }
}

impl Into<Canonicalization> for Canon {
    fn into(self) -> Canonicalization {
        match self {
            Self::Relaxed => Canonicalization::Relaxed,
            Self::Simple => Canonicalization::Simple,
        }
    }
}

#[derive(Deserialize, Hash, Eq, PartialEq, Copy, Clone)]
pub enum HashAlgo {
    Sha1,
    Sha256,
}

impl Into<HashAlgorithm> for HashAlgo {
    fn into(self) -> HashAlgorithm {
        match self {
            Self::Sha1 => HashAlgorithm::Sha1,
            Self::Sha256 => HashAlgorithm::Sha256,
        }
    }
}

#[derive(Deserialize, Hash, PartialEq, Eq, Clone)]
pub struct SignerConfig {
    domain: String,
    selector: String,
    headers: Vec<String>,
    #[serde(default)]
    atps: Option<String>,
    #[serde(default)]
    atpsh: Option<HashAlgo>,
    #[serde(default)]
    agent_user_identifier: Option<String>,
    #[serde(default)]
    expiration: Option<u64>,
    #[serde(default)]
    body_length: bool,
    #[serde(default)]
    reporting: bool,
    #[serde(default)]
    header_canonicalization: Canon,
    #[serde(default)]
    body_canonicalization: Canon,

    #[serde(default)]
    use_cf: bool,

    key: KeySource,

    #[serde(default = "SignerConfig::default_ttl")]
    ttl: u64,
}

impl SignerConfig {
    fn default_ttl() -> u64 {
        300
    }

    fn configure_signer<T: SigningKey>(
        &self,
        signer: DkimSigner<T, NeedDomain>,
    ) -> DkimSigner<T, Done> {
        let mut signer = signer
            .domain(self.domain.clone())
            .selector(self.selector.clone())
            .headers(self.headers.clone());
        if let Some(atps) = &self.atps {
            signer = signer.atps(atps.clone());
        }
        if let Some(atpsh) = self.atpsh {
            signer = signer.atpsh(atpsh.into());
        }
        if let Some(agent_user_identifier) = &self.agent_user_identifier {
            signer = signer.agent_user_identifier(agent_user_identifier);
        }
        if let Some(expiration) = self.expiration {
            signer = signer.expiration(expiration);
        }
        signer = signer.body_length(self.body_length);
        signer = signer.reporting(self.reporting);
        signer = signer.header_canonicalization(self.header_canonicalization.into());
        signer = signer.body_canonicalization(self.body_canonicalization.into());

        signer
    }
}

pub enum SignerInner {
    RsaSha256(DkimSigner<RsaKey<Sha256>, Done>),
    Ed25519(DkimSigner<Ed25519Key, Done>),
    CFDKIM(CFSigner),
}

#[derive(Clone)]
pub struct Signer(Arc<SignerInner>);

impl Signer {
    pub fn sign(&self, message: &[u8]) -> anyhow::Result<String> {
        self.0.sign(message)
    }
}

impl SignerInner {
    fn sign(&self, message: &[u8]) -> anyhow::Result<String> {
        let sig = match self {
            Self::RsaSha256(signer) => signer.sign(message),
            Self::Ed25519(signer) => signer.sign(message),
            Self::CFDKIM(signer) => return signer.sign(message),
        }
        .map_err(|err| anyhow::anyhow!("{err:#}"))?;

        let header = sig.to_header();
        Ok(header)
    }
}

impl LuaUserData for Signer {}

pub fn register<'lua>(lua: &'lua Lua) -> anyhow::Result<()> {
    let dkim_mod = get_or_create_sub_module(lua, "dkim")?;
    dkim_mod.set(
        "rsa_sha256_signer",
        lua.create_async_function(|lua, params: Value| async move {
            let params: SignerConfig = lua.from_value(params)?;

            if let Some(inner) = SIGNER_CACHE.get(&params) {
                return Ok(Signer(inner));
            }

            let data = params
                .key
                .get()
                .await
                .map_err(|err| mlua::Error::external(format!("{:?}: {err:#}", params.key)))?;

            let data = String::from_utf8_lossy(&data);

            let mut errors = vec![];

            let inner;

            if params.use_cf {
                // Verify that we can parse it immediately

                let key_data = CFKeyData::guess_type(&data)
                    .map_err(|err| mlua::Error::external(format!("{:?}: {err}", params.key)))?;

                inner = Arc::new(SignerInner::CFDKIM(CFSigner {
                    config: params.clone(),
                    key_data,
                }));
            } else {
                let key = RsaKey::<Sha256>::from_rsa_pem(&data)
                    .or_else(|err| {
                        errors.push(format!("from_rsa_pem: {err:#}"));
                        RsaKey::<Sha256>::from_pkcs8_pem(&data)
                    })
                    .map_err(|err| {
                        let err = format!("from_pkcs8_pem: {err:#}");
                        errors.push(err.clone());
                        if err.contains("TooSmall") {
                            // <https://docs.rs/ring/latest/ring/signature/struct.RsaKeyPair.html#method.from_pkcs8>
                            // Technically 2047 or larger, but recommend 2048 or 3072
                            errors.push(format!(
                                "Note: This implementation supports \
                             RSA keys that are 2048 bits or larger"
                            ));
                        }
                        mlua::Error::external(format!("{:?}: {}", params.key, errors.join(", ")))
                    })?;

                let signer = params.configure_signer(DkimSigner::from_key(key));

                inner = Arc::new(SignerInner::RsaSha256(signer));
            };

            let expiration = Instant::now() + Duration::from_secs(params.ttl);
            SIGNER_CACHE.insert(params, Arc::clone(&inner), expiration);

            Ok(Signer(inner))
        })?,
    )?;

    dkim_mod.set(
        "ed25519_signer",
        lua.create_async_function(|lua, params: Value| async move {
            let params: SignerConfig = lua.from_value(params)?;

            if let Some(inner) = SIGNER_CACHE.get(&params) {
                return Ok(Signer(inner));
            }

            let data = params
                .key
                .get()
                .await
                .map_err(|err| mlua::Error::external(format!("{:?}: {err:#}", params.key)))?;

            let mut errors = vec![];

            let key = Ed25519Key::from_pkcs8_der(&data)
                .or_else(|err| {
                    errors.push(format!("from_pkcs8_der: {err:#}"));
                    Ed25519Key::from_pkcs8_maybe_unchecked_der(&data)
                })
                .map_err(|err| {
                    errors.push(format!("from_pkcs8_maybe_unchecked_der: {err:#}"));
                    mlua::Error::external(format!("{:?}: {}", params.key, errors.join(", ")))
                })?;

            let signer = params.configure_signer(DkimSigner::from_key(key));

            let inner = Arc::new(SignerInner::Ed25519(signer));
            let expiration = Instant::now() + Duration::from_secs(params.ttl);
            SIGNER_CACHE.insert(params, Arc::clone(&inner), expiration);

            Ok(Signer(inner))
        })?,
    )?;
    Ok(())
}

enum CFKeyData {
    RsaPemPkcs1(String),
    RsaDerPkcs1(String),
    RsaPemPkcs8(String),
    RsaDerPkcs8(String),
}

impl CFKeyData {
    fn guess_type(data: &str) -> anyhow::Result<Self> {
        let mut errors = vec![];

        match RsaPrivateKey::from_pkcs1_pem(data) {
            Ok(_) => return Ok(Self::RsaPemPkcs1(data.to_string())),
            Err(err) => errors.push(format!("RsaPrivateKey::from_pkcs1_pem: {err:#}")),
        };

        match RsaPrivateKey::from_pkcs1_der(data.as_bytes()) {
            Ok(_) => return Ok(Self::RsaDerPkcs1(data.to_string())),
            Err(err) => errors.push(format!("RsaPrivateKey::from_pkcs1_der: {err:#}")),
        };

        match RsaPrivateKey::from_pkcs8_pem(data) {
            Ok(_) => return Ok(Self::RsaPemPkcs8(data.to_string())),
            Err(err) => errors.push(format!("RsaPrivateKey::from_pkcs8_pem: {err:#}")),
        };

        match RsaPrivateKey::from_pkcs8_der(data.as_bytes()) {
            Ok(_) => return Ok(Self::RsaDerPkcs8(data.to_string())),
            Err(err) => errors.push(format!("RsaPrivateKey::from_pkcs8_der: {err:#}")),
        };

        anyhow::bail!("{}", errors.join(", "));
    }

    fn to_key(&self) -> anyhow::Result<cfdkim::DkimPrivateKey> {
        match self {
            Self::RsaPemPkcs1(data) => Ok(DkimPrivateKey::Rsa(
                RsaPrivateKey::from_pkcs1_pem(&data).context("RsaPrivateKey::from_pkcs1_pem")?,
            )),
            Self::RsaDerPkcs1(data) => Ok(DkimPrivateKey::Rsa(
                RsaPrivateKey::from_pkcs1_der(data.as_bytes())
                    .context("RsaPrivateKey::from_pkcs1_der")?,
            )),
            Self::RsaPemPkcs8(data) => Ok(DkimPrivateKey::Rsa(
                RsaPrivateKey::from_pkcs8_pem(&data).context("RsaPrivateKey::from_pkcs8_pem")?,
            )),
            Self::RsaDerPkcs8(data) => Ok(DkimPrivateKey::Rsa(
                RsaPrivateKey::from_pkcs8_der(data.as_bytes())
                    .context("RsaPrivateKey::from_pkcs8_der")?,
            )),
        }
    }
}

pub struct CFSigner {
    config: SignerConfig,
    key_data: CFKeyData,
}

impl CFSigner {
    fn sign(&self, message: &[u8]) -> anyhow::Result<String> {
        let key = self.key_data.to_key()?;

        let headers: Vec<&str> = self.config.headers.iter().map(|s| s.as_str()).collect();
        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let mut signer = cfdkim::SignerBuilder::new()
            .with_signed_headers(&headers)
            .context("configure signed headers")?
            .with_private_key(key)
            .with_selector(&self.config.selector)
            .with_signing_domain(&self.config.domain)
            .with_logger(&logger)
            .with_header_canonicalization(match self.config.header_canonicalization {
                Canon::Relaxed => cfdkim::canonicalization::Type::Relaxed,
                Canon::Simple => cfdkim::canonicalization::Type::Simple,
            })
            .with_body_canonicalization(match self.config.body_canonicalization {
                Canon::Relaxed => cfdkim::canonicalization::Type::Relaxed,
                Canon::Simple => cfdkim::canonicalization::Type::Simple,
            });
        if let Some(exp) = self.config.expiration {
            signer = signer.with_expiry(chrono::Duration::seconds(exp as i64));
        }

        let signer = signer.build().context("build signer")?;

        let mail = mailparse::parse_mail(message).context("parsing message")?;

        let dkim_header = signer.sign(&mail)?;

        Ok(dkim_header)
    }
}
