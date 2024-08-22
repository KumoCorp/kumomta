use anyhow::Context;
use config::{from_lua_value, get_or_create_sub_module};
use data_loader::KeySource;
use kumo_dkim::DkimPrivateKey;
use lruttl::LruCacheWithTtl;
use mlua::prelude::LuaUserData;
use mlua::{Lua, Value};
use prometheus::{Counter, Histogram};
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

lazy_static::lazy_static! {
    static ref SIGNER_CACHE: LruCacheWithTtl<SignerConfig, Arc<CFSigner>> = LruCacheWithTtl::new(1024);
    static ref SIGNER_KEY_FETCH: Histogram = prometheus::register_histogram!(
        "dkim_signer_key_fetch",
        "how long it takes to obtain a dkim key").unwrap();
    static ref SIGNER_CREATE: Histogram = prometheus::register_histogram!(
        "dkim_signer_creation",
        "how long it takes to create a signer on a cache miss").unwrap();
    static ref SIGNER_SIGN: Histogram = prometheus::register_histogram!(
        "dkim_signer_sign",
        "how long it takes to dkim sign parsed messages").unwrap();
    static ref SIGNER_PARSE: Histogram = prometheus::register_histogram!(
        "dkim_signer_message_parse",
        "how long it takes to parse messages as prep for signing").unwrap();
    static ref SIGNER_CACHE_HIT: Counter = prometheus::register_counter!(
        "dkim_signer_cache_hit",
        "how many cache dkim signer requests hit cache").unwrap();
    static ref SIGNER_CACHE_MISS: Counter = prometheus::register_counter!(
        "dkim_signer_cache_miss",
        "how many cache dkim signer requests miss cache").unwrap();
    static ref SIGNER_CACHE_LOOKUP: Counter = prometheus::register_counter!(
        "dkim_signer_cache_lookup_count",
        "how many cache dkim signer requests occurred").unwrap();
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

#[derive(Deserialize, Hash, Eq, PartialEq, Copy, Clone)]
pub enum HashAlgo {
    Sha1,
    Sha256,
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
    #[serde(default)]
    over_sign: bool,

    #[serde(default = "SignerConfig::default_ttl")]
    ttl: u64,
}

impl SignerConfig {
    fn default_ttl() -> u64 {
        300
    }

    fn configure_kumo_dkim(&self, key: DkimPrivateKey) -> anyhow::Result<kumo_dkim::Signer> {
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

        let mut signer = kumo_dkim::SignerBuilder::new()
            .with_signed_headers(&self.headers)
            .context("configure signed headers")?
            .with_private_key(key)
            .with_selector(&self.selector)
            .with_signing_domain(&self.domain)
            .with_over_signing(self.over_sign)
            .with_header_canonicalization(match self.header_canonicalization {
                Canon::Relaxed => kumo_dkim::canonicalization::Type::Relaxed,
                Canon::Simple => kumo_dkim::canonicalization::Type::Simple,
            })
            .with_body_canonicalization(match self.body_canonicalization {
                Canon::Relaxed => kumo_dkim::canonicalization::Type::Relaxed,
                Canon::Simple => kumo_dkim::canonicalization::Type::Simple,
            });
        if let Some(exp) = self.expiration {
            signer =
                signer.with_expiry(chrono::Duration::try_seconds(exp as i64).ok_or_else(|| {
                    anyhow::anyhow!("{exp} is out of range for chrono::Duration::try_seconds")
                })?);
        }

        signer.build().context("build signer")
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "impl", derive(mlua::FromLua))]
pub struct Signer(Arc<CFSigner>);

impl Signer {
    pub fn sign(&self, message: &[u8]) -> anyhow::Result<String> {
        self.0.sign(message)
    }
}

impl LuaUserData for Signer {}

pub fn register<'lua>(lua: &'lua Lua) -> anyhow::Result<()> {
    let dkim_mod = get_or_create_sub_module(lua, "dkim")?;
    dkim_mod.set(
        "rsa_sha256_signer",
        lua.create_async_function(|lua, params: Value| async move {
            let params: SignerConfig = from_lua_value(lua, params)?;

            SIGNER_CACHE_LOOKUP.inc();
            if let Some(inner) = SIGNER_CACHE.get(&params) {
                SIGNER_CACHE_HIT.inc();
                return Ok(Signer(inner));
            }
            SIGNER_CACHE_MISS.inc();

            let signer_creation_timer = SIGNER_CREATE.start_timer();
            let key_fetch_timer = SIGNER_KEY_FETCH.start_timer();
            let data = params
                .key
                .get()
                .await
                .map_err(|err| mlua::Error::external(format!("{:?}: {err:#}", params.key)))?;

            let key = DkimPrivateKey::rsa_key(&data)
                .map_err(|err| mlua::Error::external(format!("{:?}: {err}", params.key)))?;
            key_fetch_timer.stop_and_record();

            let signer = params
                .configure_kumo_dkim(key)
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?;

            let inner = Arc::new(CFSigner { signer });

            let expiration = Instant::now() + Duration::from_secs(params.ttl);
            SIGNER_CACHE.insert(params, Arc::clone(&inner), expiration);

            signer_creation_timer.stop_and_record();
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

            let signer_creation_timer = SIGNER_CREATE.start_timer();
            let key_fetch_timer = SIGNER_KEY_FETCH.start_timer();
            let data = params
                .key
                .get()
                .await
                .map_err(|err| mlua::Error::external(format!("{:?}: {err:#}", params.key)))?;

            let key = DkimPrivateKey::ed25519_key(&data)
                .map_err(|err| mlua::Error::external(format!("{:?}: {err}", params.key)))?;
            key_fetch_timer.stop_and_record();

            let signer = params
                .configure_kumo_dkim(key)
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?;

            let inner = Arc::new(CFSigner { signer });

            let expiration = Instant::now() + Duration::from_secs(params.ttl);
            SIGNER_CACHE.insert(params, Arc::clone(&inner), expiration);

            signer_creation_timer.stop_and_record();
            Ok(Signer(inner))
        })?,
    )?;
    Ok(())
}

pub struct CFSigner {
    signer: kumo_dkim::Signer,
}

impl CFSigner {
    fn sign(&self, message: &[u8]) -> anyhow::Result<String> {
        let parse_timer = SIGNER_PARSE.start_timer();
        let message_str =
            std::str::from_utf8(message).context("DKIM signer: message is not ASCII or UTF-8")?;
        let mail = kumo_dkim::ParsedEmail::parse(message_str)
            .context("failed to parse message to pass to dkim signer")?;
        parse_timer.stop_and_record();

        let sign_timer = SIGNER_SIGN.start_timer();
        let dkim_header = self.signer.sign(&mail)?;
        sign_timer.stop_and_record();

        Ok(dkim_header)
    }
}
