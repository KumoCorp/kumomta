use config::{any_err, from_lua_value, get_or_create_sub_module};
use data_loader::KeySource;
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, UserDataMethods, Value};
use rspamd_client::config::{Config, EnvelopeData};
use rspamd_client::protocol::RspamdScanReply;
use rspamd_client::scan_async;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

/// Configuration for Rspamd client (Lua-friendly wrapper)
#[derive(Deserialize, Debug, Clone)]
struct RspamdClientConfig {
    /// Base URL for Rspamd server (e.g., "http://localhost:11333")
    base_url: String,

    /// Optional password for Rspamd authentication
    #[serde(default)]
    password: Option<String>,

    /// Optional timeout in seconds (default: 30.0)
    #[serde(default, with = "duration_serde")]
    timeout: Option<std::time::Duration>,

    /// Number of retries for requests (default: 1)
    #[serde(default)]
    retries: Option<u32>,

    /// Enable ZSTD compression (default: true)
    /// Automatically compresses message bodies for faster transmission
    #[serde(default = "default_true")]
    zstd: bool,

    /// Optional HTTPCrypt encryption key (base32 format)
    /// When set, enables encrypted communication with Rspamd
    /// Generate with: rspamadm keypair
    #[serde(default)]
    encryption_key: Option<String>,

    /// Optional proxy URL (e.g., "http://proxy.example.com:8080")
    #[serde(default)]
    proxy_url: Option<String>,

    /// Optional proxy username
    #[serde(default)]
    proxy_username: Option<String>,

    /// Optional proxy password
    #[serde(default)]
    proxy_password: Option<String>,

    /// Optional TLS certificate
    #[serde(default)]
    tls_certificate: Option<KeySource>,

    /// Optional TLS private key
    #[serde(default)]
    tls_private_key: Option<KeySource>,

    /// Optional TLS CA certificate
    #[serde(default)]
    tls_ca_certificate: Option<KeySource>,
}

fn default_true() -> bool {
    true
}

/// Wrapper around rspamd-client Config for Lua
#[derive(Clone)]
struct RspamdClient {
    config: Arc<Config>,
}

impl RspamdClient {
    fn new(lua_config: RspamdClientConfig) -> anyhow::Result<Self> {
        // Prepare optional proxy config
        let proxy_config = lua_config.proxy_url.map(|proxy_url| {
            rspamd_client::config::ProxyConfig {
                proxy_url,
                username: lua_config.proxy_username.clone(),
                password: lua_config.proxy_password.clone(),
            }
        });

        // Extract file paths from KeySource
        // For now, we only support File variant; Data/Vault would require temporary files
        let extract_path = |key_source: &KeySource, param_name: &str| -> anyhow::Result<String> {
            match key_source {
                KeySource::File(path) => Ok(path.clone()),
                KeySource::Data { .. } => {
                    anyhow::bail!(
                        "{} does not support inline data; please use a file path. \
                        Consider writing the data to a file first.",
                        param_name
                    )
                }
                KeySource::Vault { .. } => {
                    anyhow::bail!(
                        "{} does not support Vault secrets; please use a file path. \
                        Consider using a file-based secret management approach.",
                        param_name
                    )
                }
            }
        };

        // Prepare optional TLS settings
        let tls_settings = match (&lua_config.tls_certificate, &lua_config.tls_private_key) {
            (Some(cert_source), Some(key_source)) => {
                let cert_path = extract_path(cert_source, "tls_certificate")?;
                let key_path = extract_path(key_source, "tls_private_key")?;
                let ca_path = lua_config
                    .tls_ca_certificate
                    .as_ref()
                    .map(|ca| extract_path(ca, "tls_ca_certificate"))
                    .transpose()?;

                Some(rspamd_client::config::TlsSettings {
                    cert_path,
                    key_path,
                    ca_path,
                })
            }
            _ => None,
        };

        // Construct Config directly since all fields are public
        let config = Config {
            base_url: lua_config.base_url,
            password: lua_config.password,
            timeout: lua_config.timeout.map(|t| t.as_secs_f64()).unwrap_or(30.0),
            retries: lua_config.retries.unwrap_or(1),
            tls_settings,
            proxy_config,
            zstd: lua_config.zstd,
            encryption_key: lua_config.encryption_key,
        };

        Ok(Self {
            config: Arc::new(config),
        })
    }

    fn get_config(&self) -> mlua::Result<&Config> {
        Ok(&self.config)
    }

    async fn scan(
        &self,
        message: String,
        envelope: EnvelopeDataLua,
    ) -> anyhow::Result<RspamdScanReply> {
        let config = self.get_config().map_err(|e| anyhow::anyhow!("{}", e))?;
        let envelope_data = envelope.into_envelope_data();
        let result = scan_async(config, message, envelope_data).await?;
        Ok(result)
    }
}

impl LuaUserData for RspamdClient {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Main scan method
        methods.add_async_method(
            "scan",
            |lua, this, (message, metadata): (String, Option<EnvelopeDataLua>)| async move {
                let envelope = metadata.unwrap_or_default();

                let result = this.scan(message, envelope).await.map_err(any_err)?;

                lua.to_value(&result)
            },
        );
    }
}

/// Lua-friendly envelope data structure
#[derive(Debug, Default, Clone, mlua::FromLua)]
struct EnvelopeDataLua {
    from: Option<String>,
    rcpt: Vec<String>,
    ip: Option<String>,
    user: Option<String>,
    helo: Option<String>,
    hostname: Option<String>,
    queue_id: Option<String>,
    file_path: Option<String>,
    body_block: bool,
    additional_headers: Option<HashMap<String, String>>,
}

impl EnvelopeDataLua {
    fn into_envelope_data(self) -> EnvelopeData {
        // Build additional headers, adding queue_id if present
        let mut additional_headers = self.additional_headers.unwrap_or_default();
        if let Some(queue_id) = self.queue_id {
            additional_headers.insert("Queue-Id".to_string(), queue_id);
        }

        // Construct EnvelopeData directly since all fields are public
        EnvelopeData {
            from: self.from,
            rcpt: self.rcpt,
            ip: self.ip,
            user: self.user,
            helo: self.helo,
            hostname: self.hostname,
            file_path: self.file_path,
            body_block: self.body_block,
            additional_headers,
        }
    }
}

/// Register the rspamd module with Lua
pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let rspamd_mod = get_or_create_sub_module(lua, "rspamd")?;

    rspamd_mod.set(
        "build_client",
        lua.create_function(|lua, options: Value| {
            let config: RspamdClientConfig = from_lua_value(lua, options)?;
            RspamdClient::new(config).map_err(any_err)
        })?,
    )?;

    Ok(())
}
