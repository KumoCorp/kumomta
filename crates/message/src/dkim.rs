use anyhow::Context;
use config::{any_err, from_lua_value, get_or_create_sub_module};
use data_loader::KeySource;
use kumo_dkim::DkimPrivateKey;
use lruttl::declare_cache;
use mlua::prelude::LuaUserData;
use mlua::{Lua, Value};
use prometheus::{Counter, Histogram};
use serde::Deserialize;
use std::sync::{Arc, LazyLock, OnceLock};
use tokio::runtime::Runtime;
use tokio::time::{Duration, Instant};

declare_cache! {
static SIGNER_CACHE: LruCacheWithTtl<SignerConfig, Arc<CFSigner>>::new("dkim_signer_cache", 1024);
}
declare_cache! {
static KEY_CACHE: LruCacheWithTtl<KeySource, Arc<DkimPrivateKey>>::new("dkim_key_cache", 1024);
}

static SIGNER_KEY_FETCH: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "dkim_signer_key_fetch",
        "how long it takes to obtain a dkim key"
    )
    .unwrap()
});
static SIGNER_CREATE: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "dkim_signer_creation",
        "how long it takes to create a signer on a cache miss"
    )
    .unwrap()
});
static SIGNER_SIGN: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "dkim_signer_sign",
        "how long it takes to dkim sign parsed messages"
    )
    .unwrap()
});
static SIGNER_PARSE: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "dkim_signer_message_parse",
        "how long it takes to parse messages as prep for signing"
    )
    .unwrap()
});
static SIGNER_CACHE_HIT: LazyLock<Counter> = LazyLock::new(|| {
    prometheus::register_counter!(
        "dkim_signer_cache_hit",
        "how many cache dkim signer requests hit cache"
    )
    .unwrap()
});
static SIGNER_CACHE_MISS: LazyLock<Counter> = LazyLock::new(|| {
    prometheus::register_counter!(
        "dkim_signer_cache_miss",
        "how many cache dkim signer requests miss cache"
    )
    .unwrap()
});
static SIGNER_CACHE_LOOKUP: LazyLock<Counter> = LazyLock::new(|| {
    prometheus::register_counter!(
        "dkim_signer_cache_lookup_count",
        "how many cache dkim signer requests occurred"
    )
    .unwrap()
});

static KEY_CACHE_HIT: LazyLock<Counter> = LazyLock::new(|| {
    prometheus::register_counter!(
        "dkim_signer_key_cache_hit",
        "how many cache dkim signer requests hit key cache"
    )
    .unwrap()
});
static KEY_CACHE_MISS: LazyLock<Counter> = LazyLock::new(|| {
    prometheus::register_counter!(
        "dkim_signer_key_cache_miss",
        "how many cache dkim signer requests miss key cache"
    )
    .unwrap()
});
static KEY_CACHE_LOOKUP: LazyLock<Counter> = LazyLock::new(|| {
    prometheus::register_counter!(
        "dkim_signer_key_cache_lookup_count",
        "how many cache dkim key requests occurred"
    )
    .unwrap()
});

#[derive(Deserialize, Hash, Eq, PartialEq, Copy, Clone, Debug)]
pub enum Canon {
    Relaxed,
    Simple,
}

impl Default for Canon {
    fn default() -> Self {
        Self::Relaxed
    }
}

#[derive(Deserialize, Hash, Eq, PartialEq, Copy, Clone, Debug)]
pub enum HashAlgo {
    Sha1,
    Sha256,
}

#[derive(Deserialize, Hash, PartialEq, Eq, Clone, Debug)]
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

    #[serde(default = "SignerConfig::default_ttl", with = "duration_serde")]
    ttl: Duration,
}

impl SignerConfig {
    fn default_ttl() -> Duration {
        Duration::from_secs(300)
    }

    fn configure_kumo_dkim(&self, key: Arc<DkimPrivateKey>) -> anyhow::Result<kumo_dkim::Signer> {
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

pub static SIGN_POOL: OnceLock<Runtime> = OnceLock::new();

#[derive(Clone)]
#[cfg_attr(feature = "impl", derive(mlua::FromLua))]
pub struct Signer(Arc<CFSigner>);

impl Signer {
    pub fn sign(&self, message: &[u8]) -> anyhow::Result<String> {
        self.0.sign(message)
    }
}

impl LuaUserData for Signer {}

async fn cached_key_load(key: &KeySource, ttl: Duration) -> anyhow::Result<Arc<DkimPrivateKey>> {
    KEY_CACHE_LOOKUP.inc();
    if let Some(pkey) = KEY_CACHE.get(key).await {
        KEY_CACHE_HIT.inc();
        return Ok(pkey);
    }

    KEY_CACHE_MISS.inc();
    let key_fetch_timer = SIGNER_KEY_FETCH.start_timer();
    let data = key.get().await?;

    let pkey = Arc::new(DkimPrivateKey::rsa_key(&data)?);
    key_fetch_timer.stop_and_record();

    KEY_CACHE
        .insert(key.clone(), pkey.clone(), Instant::now() + ttl)
        .await;
    Ok(pkey)
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let dkim_mod = get_or_create_sub_module(lua, "dkim")?;
    dkim_mod.set(
        "set_signing_threads",
        lua.create_function(move |_lua, n: usize| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .thread_name("dkimsign")
                .worker_threads(1)
                .max_blocking_threads(n)
                .build()
                .map_err(any_err)?;
            SIGN_POOL
                .set(runtime)
                .map_err(|_| mlua::Error::external("dkimsign pool is already configured"))?;
            println!("started dkimsign pool with {n} threads");
            Ok(())
        })?,
    )?;
    dkim_mod.set(
        "rsa_sha256_signer",
        lua.create_async_function(|lua, params: Value| async move {
            let params: SignerConfig = from_lua_value(&lua, params)?;

            SIGNER_CACHE_LOOKUP.inc();
            if let Some(inner) = SIGNER_CACHE.get(&params).await {
                SIGNER_CACHE_HIT.inc();
                return Ok(Signer(inner));
            }
            SIGNER_CACHE_MISS.inc();

            let signer_creation_timer = SIGNER_CREATE.start_timer();

            let key = cached_key_load(&params.key, params.ttl)
                .await
                .map_err(|err| mlua::Error::external(format!("{:?}: {err:#}", params.key)))?;

            let signer = params
                .configure_kumo_dkim(key)
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?;

            let inner = Arc::new(CFSigner { signer });

            let expiration = Instant::now() + params.ttl;
            SIGNER_CACHE
                .insert(params, Arc::clone(&inner), expiration)
                .await;

            signer_creation_timer.stop_and_record();
            Ok(Signer(inner))
        })?,
    )?;

    dkim_mod.set(
        "ed25519_signer",
        lua.create_async_function(|lua, params: Value| async move {
            let params: SignerConfig = from_lua_value(&lua, params)?;

            if let Some(inner) = SIGNER_CACHE.get(&params).await {
                return Ok(Signer(inner));
            }

            let signer_creation_timer = SIGNER_CREATE.start_timer();

            let key = cached_key_load(&params.key, params.ttl)
                .await
                .map_err(|err| mlua::Error::external(format!("{:?}: {err:#}", params.key)))?;

            let signer = params
                .configure_kumo_dkim(key)
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?;

            let inner = Arc::new(CFSigner { signer });

            let expiration = Instant::now() + params.ttl;
            SIGNER_CACHE
                .insert(params, Arc::clone(&inner), expiration)
                .await;

            signer_creation_timer.stop_and_record();
            Ok(Signer(inner))
        })?,
    )?;
    Ok(())
}

#[derive(Debug)]
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
