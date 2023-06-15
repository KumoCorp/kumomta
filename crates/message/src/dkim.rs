use anyhow::Context;
use cfdkim::DkimPrivateKey;
use config::{from_lua_value, get_or_create_sub_module};
use data_loader::KeySource;
use lruttl::LruCacheWithTtl;
use mail_auth::common::crypto::{Ed25519Key, HashAlgorithm, RsaKey, Sha256, SigningKey};
use mail_auth::common::headers::HeaderWriter;
use mail_auth::dkim::{Canonicalization, DkimSigner, Done, NeedDomain};
use mlua::prelude::LuaUserData;
use mlua::{Lua, Value};
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

    fn configure_cfdkim(&self, key: DkimPrivateKey) -> anyhow::Result<cfdkim::Signer> {
        if self.atps.is_some() {
            anyhow::bail!("atps is not currently supported for RSA keys");
        }
        if self.atpsh.is_some() {
            anyhow::bail!("atpsh is not currently supported for RSA keys");
        }
        if self.agent_user_identifier.is_some() {
            anyhow::bail!("agent_user_identifier is not currently supported for RSA keys");
        }
        if self.body_length {
            anyhow::bail!("body_length is not currently supported for RSA keys");
        }
        if self.reporting {
            anyhow::bail!("reporting is not currently supported for RSA keys");
        }

        let mut signer = cfdkim::SignerBuilder::new()
            .with_signed_headers(&self.headers)
            .context("configure signed headers")?
            .with_private_key(key)
            .with_selector(&self.selector)
            .with_signing_domain(&self.domain)
            .with_header_canonicalization(match self.header_canonicalization {
                Canon::Relaxed => cfdkim::canonicalization::Type::Relaxed,
                Canon::Simple => cfdkim::canonicalization::Type::Simple,
            })
            .with_body_canonicalization(match self.body_canonicalization {
                Canon::Relaxed => cfdkim::canonicalization::Type::Relaxed,
                Canon::Simple => cfdkim::canonicalization::Type::Simple,
            });
        if let Some(exp) = self.expiration {
            signer = signer.with_expiry(chrono::Duration::seconds(exp as i64));
        }

        signer.build().context("build signer")
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
            let params: SignerConfig = from_lua_value(lua, params)?;

            if let Some(inner) = SIGNER_CACHE.get(&params) {
                return Ok(Signer(inner));
            }

            let data = params
                .key
                .get()
                .await
                .map_err(|err| mlua::Error::external(format!("{:?}: {err:#}", params.key)))?;

            let data = String::from_utf8_lossy(&data);

            let key = load_dkim_rsa_key(&data)
                .map_err(|err| mlua::Error::external(format!("{:?}: {err}", params.key)))?;

            let signer = params
                .configure_cfdkim(key)
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?;

            let inner = Arc::new(SignerInner::CFDKIM(CFSigner { signer }));

            let expiration = Instant::now() + Duration::from_secs(params.ttl);
            SIGNER_CACHE.insert(params, Arc::clone(&inner), expiration);

            Ok(Signer(inner))
        })?,
    )?;

    dkim_mod.set(
        "ed25519_signer",
        lua.create_async_function(|lua, params: Value| async move {
            let params: SignerConfig = from_lua_value(lua, params)?;

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

fn load_dkim_rsa_key(data: &str) -> anyhow::Result<DkimPrivateKey> {
    let mut errors = vec![];

    match RsaPrivateKey::from_pkcs1_pem(data) {
        Ok(key) => return Ok(DkimPrivateKey::Rsa(key)),
        Err(err) => errors.push(format!("RsaPrivateKey::from_pkcs1_pem: {err:#}")),
    };

    match RsaPrivateKey::from_pkcs1_der(data.as_bytes()) {
        Ok(key) => return Ok(DkimPrivateKey::Rsa(key)),
        Err(err) => errors.push(format!("RsaPrivateKey::from_pkcs1_der: {err:#}")),
    };

    match RsaPrivateKey::from_pkcs8_pem(data) {
        Ok(key) => return Ok(DkimPrivateKey::Rsa(key)),
        Err(err) => errors.push(format!("RsaPrivateKey::from_pkcs8_pem: {err:#}")),
    };

    match RsaPrivateKey::from_pkcs8_der(data.as_bytes()) {
        Ok(key) => return Ok(DkimPrivateKey::Rsa(key)),
        Err(err) => errors.push(format!("RsaPrivateKey::from_pkcs8_der: {err:#}")),
    };

    anyhow::bail!("{}", errors.join(", "));
}

pub struct CFSigner {
    signer: cfdkim::Signer,
}

impl CFSigner {
    fn sign(&self, message: &[u8]) -> anyhow::Result<String> {
        let mail = mailparse::parse_mail(message).context("parsing message")?;

        let dkim_header = self.signer.sign(&mail)?;

        Ok(dkim_header)
    }
}
