use async_nats::jetstream::{self, Context};
use async_nats::rustls::lock::Mutex;
use async_nats::{ConnectOptions, HeaderMap};
use config::{any_err, get_or_create_sub_module};
use data_loader::KeySource;
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, UserDataMethods, Value};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// https://docs.rs/async-nats/0.45.0/src/async_nats/options.rs.html#44
#[derive(Debug, Deserialize)]
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
    // tls_client_config: Option<rustls::ClientConfig>,
    ping_interval: Option<Duration>,
    // subscription_capacity: Option<usize>,
    client_capacity: Option<usize>,
    // event_callback: Option<CallbackArg1<Event, ()>>,
    inbox_prefix: Option<String>,
    #[serde(default, with = "duration_serde")]
    request_timeout: Option<Duration>,
    retry_on_initial_connect: Option<bool>,
    ignore_discovered_servers: Option<bool>,
    retain_servers_order: Option<bool>,
    // read_buffer_capacity: Option<u16>,
    // reconnect_delay_callback: Box<dyn Fn(usize) -> Duration + Send + Sync + 'static>,
    // auth_callback: Option<CallbackArg1<Vec<u8>, Result<Auth, AuthError>>>,
}

#[derive(Debug, Deserialize)]
struct ConfigAuth {
    username: Option<KeySource>,
    password: Option<KeySource>,
    token: Option<KeySource>,
}

// https://docs.rs/async-nats/0.45.0/src/async_nats/auth.rs.html#4
#[derive(Debug, Deserialize)]
struct Auth {
    // jwt: Option<String>,
    // nkey: Option<String>,
    // signature_callback: Option<CallbackArg1<String, Result<String, AuthError>>>,
    // signature: Option<Vec<u8>>,
    username: Option<String>,
    password: Option<String>,
    token: Option<String>,
}

#[derive(Clone)]
struct Client {
    context: Arc<Mutex<Option<Arc<Context>>>>,
}

impl Client {
    fn get_context(&self) -> mlua::Result<Arc<Context>> {
        self.context
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| mlua::Error::external("client was closed"))
    }
}

#[derive(Deserialize, Debug)]
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
    #[serde(default)]
    await_ack: bool,
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

            if message.await_ack {
                ack_fut.await.map_err(|err| any_err(err))?;
            }

            Ok(())
        });

        methods.add_async_method("close", |_lua, this, _: ()| async move {
            this.context.lock().unwrap().take();

            Ok(())
        });
    }
}

async fn get_auth(auth: &ConfigAuth) -> anyhow::Result<Auth> {
    let username = match &auth.username {
        Some(username) => Some(String::from_utf8(username.get().await?)?),
        None => None,
    };
    let password = match &auth.password {
        Some(password) => Some(String::from_utf8(password.get().await?)?),
        None => None,
    };
    let token = match &auth.token {
        Some(token) => Some(String::from_utf8(token.get().await?)?),
        None => None,
    };

    Ok(Auth {
        username,
        password,
        token,
    })
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let nats_mod = get_or_create_sub_module(lua, "nats")?;

    nats_mod.set(
        "connect",
        lua.create_async_function(|lua, value: Value| async move {
            let config: Config = lua.from_value(value)?;
            let auth = match &config.auth {
                Some(config) => Some(get_auth(&config).await.map_err(|err| any_err(err))?),
                None => None,
            };
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
            if let Some(auth) = auth {
                if let Some(token) = auth.token {
                    opts = opts.token(token);
                }
                if let (Some(username), Some(password)) = (auth.username, auth.password) {
                    opts = opts.user_and_password(username, password);
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
            if let (Some(client_cert), Some(client_key)) = (config.client_cert, config.client_key) {
                opts = opts.add_client_certificate(client_cert, client_key);
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
            if let Some(request_timeout) = config.request_timeout {
                opts = opts.request_timeout(Some(request_timeout));
            }
            if let Some(true) = config.retry_on_initial_connect {
                opts = opts.retry_on_initial_connect();
            }
            if let Some(true) = config.ignore_discovered_servers {
                opts = opts.ignore_discovered_servers();
            }
            if let Some(true) = config.retain_servers_order {
                opts = opts.retain_servers_order();
            }

            let client = opts.connect(config.servers).await.map_err(any_err)?;
            let context = jetstream::new(client);

            Ok(Client {
                context: Arc::new(Mutex::new(Some(Arc::new(context)))),
            })
        })?,
    )?;

    Ok(())
}
