use anyhow::Context;
use config::{any_err, from_lua_value, get_or_create_sub_module};
use data_loader::KeySource;
use message::Message;
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
    /// Must be a valid URL
    base_url: url::Url,

    /// Optional password for Rspamd authentication
    #[serde(default)]
    password: Option<KeySource>,

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
    /// Supports KeySource for secure storage
    #[serde(default)]
    encryption_key: Option<KeySource>,

    /// Optional proxy URL (e.g., "http://proxy.example.com:8080")
    #[serde(default)]
    proxy_url: Option<String>,

    /// Optional proxy username
    /// Supports KeySource for secure storage
    #[serde(default)]
    proxy_username: Option<KeySource>,

    /// Optional proxy password
    /// Supports KeySource for secure storage
    #[serde(default)]
    proxy_password: Option<KeySource>,

    /// Optional TLS certificate
    #[serde(default)]
    tls_certificate: Option<KeySource>,

    /// Optional TLS private key
    #[serde(default)]
    tls_private_key: Option<KeySource>,

    /// Optional TLS CA certificate
    #[serde(default)]
    tls_ca_certificate: Option<KeySource>,

    /// If true, add X-Spam-* headers to the message (default: true)
    #[serde(default = "default_true")]
    add_headers: bool,

    /// If true, prefix the Subject header with "***SPAM*** " (default: false)
    #[serde(default)]
    prefix_subject: bool,

    /// If true, reject spam messages (default: false)
    #[serde(default)]
    reject_spam: bool,

    /// If true, and reject_spam is true, use 4xx instead of 5xx rejection (default: false)
    #[serde(default)]
    reject_soft: bool,
}

fn default_true() -> bool {
    true
}

impl RspamdClientConfig {
    /// Build rspamd-client Config from this configuration
    fn make_client_config(&self) -> anyhow::Result<Config> {
        // Helper to extract file paths from KeySource (for TLS certificates)
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

        // Helper to extract string value from KeySource (for passwords/keys)
        let extract_string =
            |key_source: &KeySource, param_name: &str| -> anyhow::Result<String> {
                match key_source {
                    KeySource::File(path) => {
                        std::fs::read_to_string(path).with_context(|| {
                            format!("Failed to read {} from file: {}", param_name, path)
                        })
                    }
                    KeySource::Data { key_data, .. } => Ok(key_data.clone()),
                    KeySource::Vault { .. } => {
                        anyhow::bail!(
                            "{} does not support Vault secrets yet. \
                            Consider using a file or inline data instead.",
                            param_name
                        )
                    }
                }
            };

        // Extract password if provided
        let password = self
            .password
            .as_ref()
            .map(|p| extract_string(p, "password"))
            .transpose()?;

        // Extract encryption key if provided
        let encryption_key = self
            .encryption_key
            .as_ref()
            .map(|k| extract_string(k, "encryption_key"))
            .transpose()?;

        // Prepare optional proxy config
        let proxy_config = if let Some(proxy_url) = &self.proxy_url {
            let username = self
                .proxy_username
                .as_ref()
                .map(|u| extract_string(u, "proxy_username"))
                .transpose()?;
            let proxy_password = self
                .proxy_password
                .as_ref()
                .map(|p| extract_string(p, "proxy_password"))
                .transpose()?;

            Some(rspamd_client::config::ProxyConfig {
                proxy_url: proxy_url.clone(),
                username,
                password: proxy_password,
            })
        } else {
            None
        };

        // Prepare optional TLS settings
        let tls_settings = match (&self.tls_certificate, &self.tls_private_key) {
            (Some(cert_source), Some(key_source)) => {
                let cert_path = extract_path(cert_source, "tls_certificate")?;
                let key_path = extract_path(key_source, "tls_private_key")?;
                let ca_path = self
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
        Ok(Config {
            base_url: self.base_url.to_string(),
            password,
            timeout: self.timeout.map(|t| t.as_secs_f64()).unwrap_or(30.0),
            retries: self.retries.unwrap_or(1),
            tls_settings,
            proxy_config,
            zstd: self.zstd,
            encryption_key,
        })
    }
}

/// Wrapper around rspamd-client Config for Lua
#[derive(Clone)]
struct RspamdClient {
    config: Arc<Config>,
}

impl RspamdClient {
    fn new(lua_config: RspamdClientConfig) -> anyhow::Result<Self> {
        let config = lua_config.make_client_config()?;
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

    rspamd_mod.set(
        "scan_message",
        lua.create_async_function(|lua, (config_value, msg): (Value, Message)| async move {
            let config: RspamdClientConfig = from_lua_value(&lua, config_value)?;

            // Extract envelope data from message
            let envelope_data = EnvelopeData {
                from: msg.sender().map(|s| s.to_string()).ok(),
                rcpt: msg.recipient_list_string().unwrap_or_default(),
                ip: msg.get_meta_string("received_from").ok().flatten(),
                user: msg.get_meta_string("authn_id").ok().flatten(),
                helo: msg.get_meta_string("ehlo_domain").ok().flatten(),
                hostname: msg.get_meta_string("hostname").ok().flatten(),
                // We don't have a reliable file path in most cases
                file_path: None,
                body_block: false,
                additional_headers: HashMap::default(),
            };

            // Build client config and scan
            // Convert Arc<Box<[u8]>> to Vec<u8> for scan_async
            let message_data = msg.get_data();
            let message_bytes: Vec<u8> = message_data.as_ref().to_vec();

            let client_config = config.make_client_config().map_err(any_err)?;
            let reply = scan_async(&client_config, message_bytes, envelope_data)
                .await
                .map_err(any_err)?;

            // Apply default actions based on config
            if config.add_headers {
                // Add X-Spam-* headers
                msg.prepend_header(
                    Some("X-Spam-Flag"),
                    if reply.score > 0.0 { "YES" } else { "NO" },
                );
                msg.prepend_header(Some("X-Spam-Score"), &reply.score.to_string());
                msg.prepend_header(Some("X-Spam-Action"), &reply.action);

                // Add symbols if available
                if !reply.symbols.is_empty() {
                    let symbols: Vec<String> = reply.symbols.keys().cloned().collect();
                    msg.prepend_header(Some("X-Spam-Symbols"), &symbols.join(", "));
                }
            }

            if config.prefix_subject && reply.action == "reject" {
                // Prefix subject with SPAM marker
                if let Ok(Some(subject)) = msg.get_first_named_header_value("Subject") {
                    msg.remove_all_named_headers("Subject").map_err(any_err)?;
                    msg.prepend_header(Some("Subject"), &format!("***SPAM*** {}", subject));
                }
            }

            if config.reject_spam && reply.action == "reject" {
                // Call kumo.reject via Lua runtime
                let globals = lua.globals();
                let kumo: mlua::Table = globals.get("kumo")?;
                let reject: mlua::Function = kumo.get("reject")?;

                let code = if config.reject_soft { 451 } else { 550 };
                let message = format!(
                    "{} Spam detected (score: {:.2})",
                    if config.reject_soft { "4.7.1" } else { "5.7.1" },
                    reply.score
                );

                reject.call::<()>((code, message))?;
            }

            // Return the scan reply
            lua.to_value(&reply)
        })?,
    )?;

    Ok(())
}
