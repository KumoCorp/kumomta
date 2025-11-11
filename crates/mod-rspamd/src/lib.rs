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

/// Zero-copy wrapper for Arc<Box<[u8]>> that implements AsRef<[u8]>
/// This allows passing message data to rspamd-client without allocating or copying
struct MessageDataWrapper(Arc<Box<[u8]>>);

impl AsRef<[u8]> for MessageDataWrapper {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

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

    /// If true, add X-Spam-* headers to the message (default: true)
    /// Headers added: X-Spam-Flag, X-Spam-Score, X-Spam-Action, X-Spam-Symbols
    #[serde(default = "default_true")]
    add_headers: bool,

    /// If true, prefix the Subject header with "***SPAM*** " when action is "rewrite subject" (default: false)
    /// Note: This only applies when Rspamd returns action "rewrite subject"
    #[serde(default)]
    prefix_subject: bool,

    /// If true, reject messages when Rspamd action is "reject" (default: false)
    /// If false, messages with action "reject" will still be delivered with spam headers
    /// Note: Action "soft reject" always results in 451 temporary failure (greylisting)
    #[serde(default)]
    reject_spam: bool,
}

fn default_true() -> bool {
    true
}

impl RspamdClientConfig {
    /// Build rspamd-client Config from this configuration
    async fn make_client_config(&self) -> anyhow::Result<Config> {
        // Extract password if provided
        let password = if let Some(p) = &self.password {
            let bytes = p.get().await?;
            Some(String::from_utf8(bytes.to_vec()).context("password is not valid UTF-8")?)
        } else {
            None
        };

        // Extract encryption key if provided
        let encryption_key = if let Some(k) = &self.encryption_key {
            let bytes = k.get().await?;
            Some(String::from_utf8(bytes.to_vec()).context("encryption_key is not valid UTF-8")?)
        } else {
            None
        };

        // Prepare optional proxy config
        let proxy_config = if let Some(proxy_url) = &self.proxy_url {
            let username = if let Some(u) = &self.proxy_username {
                let bytes = u.get().await?;
                Some(
                    String::from_utf8(bytes.to_vec())
                        .context("proxy_username is not valid UTF-8")?,
                )
            } else {
                None
            };

            let proxy_password = if let Some(p) = &self.proxy_password {
                let bytes = p.get().await?;
                Some(
                    String::from_utf8(bytes.to_vec())
                        .context("proxy_password is not valid UTF-8")?,
                )
            } else {
                None
            };

            Some(rspamd_client::config::ProxyConfig {
                proxy_url: proxy_url.clone(),
                username,
                password: proxy_password,
            })
        } else {
            None
        };

        // Construct Config directly since all fields are public
        // Note: use HTTPCrypt encryption_key for secure communication
        Ok(Config {
            base_url: self.base_url.to_string(),
            password,
            timeout: self.timeout.map(|t| t.as_secs_f64()).unwrap_or(30.0),
            retries: self.retries.unwrap_or(1),
            tls_settings: None, // use httpcrypt instead of tls
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
    async fn new(lua_config: RspamdClientConfig) -> anyhow::Result<Self> {
        let config = lua_config.make_client_config().await?;
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
        let config = self.get_config().map_err(|e| anyhow::anyhow!("{e}"))?;
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
        lua.create_async_function(|lua, options: Value| async move {
            let config: RspamdClientConfig = from_lua_value(&lua, options)?;
            RspamdClient::new(config).await.map_err(any_err)
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
            // Wrap the Arc<Box<[u8]>> in MessageDataWrapper to implement AsRef<[u8]>
            // This enables true zero-copy operation (no allocation or copying of message data)
            let client_config = config.make_client_config().await.map_err(any_err)?;
            let data = MessageDataWrapper(msg.data().await.map_err(any_err)?);
            let reply = scan_async(&client_config, data, envelope_data)
                .await
                .map_err(any_err)?;

            // Apply default actions based on Rspamd's action field
            // X-Spam-Flag: YES for everything except "no action"
            let is_spam = reply.action != "no action";

            if config.add_headers {
                // Add X-Spam-* headers
                msg.prepend_header(Some("X-Spam-Flag"), if is_spam { "YES" } else { "NO" })
                    .await
                    .map_err(any_err)?;
                msg.prepend_header(Some("X-Spam-Score"), &reply.score.to_string())
                    .await
                    .map_err(any_err)?;
                msg.prepend_header(Some("X-Spam-Action"), &reply.action)
                    .await
                    .map_err(any_err)?;

                // Add symbols if available
                if !reply.symbols.is_empty() {
                    let symbols: Vec<&str> = reply.symbols.keys().map(|s| s.as_str()).collect();
                    msg.prepend_header(Some("X-Spam-Symbols"), &symbols.join(", "))
                        .await
                        .map_err(any_err)?;
                }
            }

            // Handle Rspamd actions
            match reply.action.as_str() {
                "no action" => {
                    // Ham - deliver normally
                }
                "soft reject" => {
                    // Temporary failure - greylisting
                    // Always use 451 for greylisting (sender should retry later)
                    let globals = lua.globals();
                    let kumo: mlua::Table = globals.get("kumo")?;
                    let reject: mlua::Function = kumo.get("reject")?;

                    // Use Rspamd's smtp_message if provided, otherwise use default
                    let message = reply
                        .messages
                        .get("smtp_message")
                        .map(|s| s.as_str())
                        .unwrap_or("4.7.1 Greylisted, please try again later");

                    reject.call::<()>((451, message))?;
                }
                "add header" => {
                    // Just add headers (already done above) and deliver
                }
                "rewrite subject" if config.prefix_subject => {
                    if let Ok(Some(subject)) = msg.get_first_named_header_value("Subject").await {
                        msg.remove_all_named_headers("Subject")
                            .await
                            .map_err(any_err)?;
                        msg.prepend_header(Some("Subject"), &format!("***SPAM*** {subject}"))
                            .await
                            .map_err(any_err)?;
                    }
                }
                "reject" if config.reject_spam => {
                    // Always use 550 (permanent failure) for spam
                    let globals = lua.globals();
                    let kumo: mlua::Table = globals.get("kumo")?;
                    let reject: mlua::Function = kumo.get("reject")?;

                    // Use Rspamd's smtp_message if provided, otherwise use default
                    let message = reply
                        .messages
                        .get("smtp_message")
                        .map(|s| s.as_str())
                        .unwrap_or("5.7.1 Spam message rejected");

                    reject.call::<()>((550, message))?;
                }
                _ => {
                    // Unknown action - deliver with headers
                }
            }

            // Return the scan reply
            lua.to_value(&reply)
        })?,
    )?;

    Ok(())
}
