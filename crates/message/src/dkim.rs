use config::get_or_create_sub_module;
use data_loader::KeySource;
use lruttl::LruCacheWithTtl;
use mail_auth::common::crypto::{HashAlgorithm, RsaKey, Sha256, SigningKey};
use mail_auth::dkim::{Canonicalization, DkimSigner, Done, NeedDomain, Signature};
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, Value};
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

#[derive(Deserialize, Hash, PartialEq, Eq)]
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
}

pub enum SignerInner {
    RsaSha256(DkimSigner<RsaKey<Sha256>, Done>),
}

#[derive(Clone)]
pub struct Signer(Arc<SignerInner>);

impl Signer {
    pub fn sign(&self, message: &[u8]) -> anyhow::Result<Signature> {
        self.0.sign(message)
    }
}

impl SignerInner {
    fn sign(&self, message: &[u8]) -> anyhow::Result<Signature> {
        match self {
            Self::RsaSha256(signer) => signer.sign(message),
        }
        .map_err(|err| anyhow::anyhow!("{err:#}"))
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

            let key = RsaKey::<Sha256>::from_rsa_pem(&data)
                .or_else(|_| RsaKey::<Sha256>::from_pkcs8_pem(&data))
                .map_err(|err| mlua::Error::external(format!("{:?}: {err:#}", params.key)))?;

            let signer = params.configure_signer(DkimSigner::from_key(key));

            let inner = Arc::new(SignerInner::RsaSha256(signer));
            let expiration = Instant::now() + Duration::from_secs(params.ttl);
            SIGNER_CACHE.insert(params, Arc::clone(&inner), expiration);

            Ok(Signer(inner))
        })?,
    )?;
    Ok(())
}
