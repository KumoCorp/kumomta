use async_nats::jetstream::Context;
use async_nats::{ConnectOptions, HeaderMap};
use config::{any_err, get_or_create_sub_module, SerdeWrappedValue};
use data_loader::KeySource;
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, UserDataMethods, Value};
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// https://docs.rs/async-nats/0.46.0/src/async_nats/options.rs.html#43
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    servers: Vec<String>,
    #[serde(default)]
    auth: Option<ConfigAuth>,

    name: Option<String>,
    no_echo: Option<bool>,
    max_reconnects: Option<usize>,
    #[serde(default, with = "duration_serde")]
    connection_timeout: Option<Duration>,
    tls_required: Option<bool>,
    tls_first: Option<bool>,
    certificate: Option<PathBuf>,
    client_cert: Option<PathBuf>,
    client_key: Option<PathBuf>,
    ping_interval: Option<Duration>,
    client_capacity: Option<usize>,
    inbox_prefix: Option<String>,
    #[serde(default, with = "duration_serde")]
    request_timeout: Option<Duration>,
    retry_on_initial_connect: Option<bool>,
    ignore_discovered_servers: Option<bool>,
    retain_servers_order: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigAuth {
    username: Option<KeySource>,
    password: Option<KeySource>,
    token: Option<KeySource>,
}

#[derive(Clone)]
struct Client {
    context: Arc<Mutex<Option<Arc<Context>>>>,
}

impl Client {
    fn get_context(&self) -> mlua::Result<Arc<Context>> {
        self.context
            .lock_arc()
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| mlua::Error::external("client was closed"))
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Message {
    /// Required destination subject
    subject: String,
    /// Payload
    #[serde(with = "serde_bytes")]
    payload: Vec<u8>,
    /// Optional headers
    #[serde(default)]
    headers: HashMap<String, String>,
    /// Optional acknowledgement
    #[serde(default = "default_true")]
    await_ack: bool,
}

fn default_true() -> bool {
    true
}

impl LuaUserData for Client {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("publish", |lua, this, value: Value| async move {
            let message: Message = lua.from_value(value)?;

            let mut headers = HeaderMap::new();
            for (key, v) in message.headers {
                headers.insert(key, v);
            }

            let ack_fut = this
                .get_context()?
                .publish_with_headers(message.subject, headers, message.payload.into())
                .await
                .map_err(|err| any_err(err))?;

            let ret = lua.create_table()?;

            if message.await_ack {
                let resp = ack_fut.await.map_err(|err| any_err(err))?;
                ret.set("stream", resp.stream)?;
                ret.set("value", resp.value.unwrap_or_default())?;
                ret.set("duplicate", resp.duplicate)?;
                ret.set("sequence", resp.sequence)?;
                ret.set("domain", resp.domain)?;
            }

            Ok(ret)
        });

        methods.add_async_method("close", |_lua, this, _: ()| async move {
            this.context.lock().take();

            Ok(())
        });
    }
}

async fn build_client(config: Config) -> anyhow::Result<async_nats::Client> {
    let mut opts = ConnectOptions::new();

    if let Some(name) = config.name {
        opts = opts.name(name);
    }
    if let Some(true) = config.no_echo {
        opts = opts.no_echo();
    }
    if let Some(max_reconnects) = config.max_reconnects {
        opts = opts.max_reconnects(max_reconnects);
    }
    if let Some(connection_timeout) = config.connection_timeout {
        opts = opts.connection_timeout(connection_timeout);
    }

    if let Some(auth) = &config.auth {
        match (&auth.username, &auth.password) {
            (Some(username), Some(password)) => {
                let username = String::from_utf8(username.get().await?)?;
                let password = String::from_utf8(password.get().await?)?;
                opts = opts.user_and_password(username, password);
            }
            (None, None) => {}
            _ => {
                anyhow::bail!("either specify both of username and password or neither");
            }
        }

        if let Some(token) = &auth.token {
            let token = String::from_utf8(token.get().await?)?;
            opts = opts.token(token);
        }
    }
    if let Some(tls_required) = config.tls_required {
        opts = opts.require_tls(tls_required);
    }
    if let Some(true) = config.tls_first {
        opts = opts.tls_first();
    }
    if let Some(certificate) = config.certificate {
        opts = opts.add_root_certificates(certificate);
    }

    match (config.client_cert, config.client_key) {
        (Some(client_cert), Some(client_key)) => {
            opts = opts.add_client_certificate(client_cert, client_key);
        }
        (None, None) => {}
        _ => {
            anyhow::bail!("either specify both of client_cert and client_key or neither");
        }
    }

    if let Some(ping_interval) = config.ping_interval {
        opts = opts.ping_interval(ping_interval);
    }
    if let Some(sender_capacity) = config.client_capacity {
        opts = opts.client_capacity(sender_capacity);
    }
    if let Some(inbox_prefix) = config.inbox_prefix {
        opts = opts.custom_inbox_prefix(inbox_prefix);
    }
    opts = opts.request_timeout(config.request_timeout);

    if let Some(true) = config.retry_on_initial_connect {
        opts = opts.retry_on_initial_connect();
    }
    if let Some(true) = config.ignore_discovered_servers {
        opts = opts.ignore_discovered_servers();
    }
    if let Some(true) = config.retain_servers_order {
        opts = opts.retain_servers_order();
    }

    Ok(opts.connect(config.servers).await?)
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let nats_mod = get_or_create_sub_module(lua, "nats")?;

    nats_mod.set(
        "connect",
        lua.create_async_function(|_lua, config: SerdeWrappedValue<Config>| async move {
            let client = build_client(config.0).await.map_err(any_err)?;
            let context = async_nats::jetstream::new(client);

            Ok(Client {
                context: Arc::new(Mutex::new(Some(Arc::new(context)))),
            })
        })?,
    )?;

    Ok(())
}
