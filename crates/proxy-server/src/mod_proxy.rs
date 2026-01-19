use anyhow::Context;
use config::{any_err, declare_event, from_lua_value, get_or_create_module, SerdeWrappedValue};
use data_loader::KeySource;
use kumo_server_common::http_server::auth::AuthKindResult;
use kumo_server_runtime::spawn;
use kumo_tls_helper::AsyncReadAndWrite;
use mlua::{IntoLua, Lua, LuaSerdeExt, Value};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// Parameters for starting a proxy listener
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ProxyListenerParams {
    /// Address to listen on (e.g., "127.0.0.1:1080")
    pub listen: String,

    /// Hostname for self-signed certificate generation if no cert is provided
    #[serde(default = "ProxyListenerParams::default_hostname")]
    pub hostname: String,

    /// Connection timeout in seconds
    #[serde(
        default = "ProxyListenerParams::default_timeout",
        with = "duration_serde"
    )]
    pub timeout: Duration,

    /// Whether to use splice(2) on Linux for proxied connections
    #[serde(default = "default_true")]
    pub use_splice: bool,

    /// Enable TLS for incoming connections
    #[serde(default)]
    pub use_tls: bool,

    /// TLS certificate file path
    #[serde(default)]
    pub tls_certificate: Option<KeySource>,

    /// TLS private key file path
    #[serde(default)]
    pub tls_private_key: Option<KeySource>,

    /// Require RFC 1929 username/password authentication
    #[serde(default)]
    pub require_auth: bool,
}

fn default_true() -> bool {
    true
}

impl ProxyListenerParams {
    fn default_hostname() -> String {
        gethostname::gethostname()
            .to_str()
            .unwrap_or("localhost")
            .to_string()
    }

    fn default_timeout() -> Duration {
        Duration::from_secs(60)
    }

    /// Start the proxy listener
    pub async fn start(self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(&self.listen).await?;
        let addr = listener.local_addr()?;

        let tls_acceptor = if self.use_tls {
            let config = kumo_server_common::tls_helpers::make_server_config(
                &self.hostname,
                &self.tls_private_key,
                &self.tls_certificate,
                &None,
            )
            .await?;
            Some(TlsAcceptor::from(config))
        } else {
            None
        };

        // Log the listener address - this format is depended upon by integration tests
        if self.use_tls {
            tracing::info!("proxy listener (TLS) on {addr:?}");
        } else {
            tracing::info!("proxy listener on {addr:?}");
        }

        let params = Arc::new(self);

        spawn(format!("proxy listener {addr:?}"), async move {
            if let Err(err) = Self::accept_loop(listener, params, tls_acceptor).await {
                tracing::error!("accept loop returned with error: {err:#}");
            }
        })?;

        Ok(())
    }

    async fn accept_loop(
        listener: TcpListener,
        params: Arc<Self>,
        tls_acceptor: Option<TlsAcceptor>,
    ) -> anyhow::Result<()> {
        let local_address = listener
            .local_addr()
            .context("failed to get local address")?;

        loop {
            let (socket, peer_address) = listener.accept().await.context("accept failed")?;

            let params = params.clone();
            let tls_acceptor = tls_acceptor.clone();

            tokio::spawn(async move {
                let result: anyhow::Result<()> = async {
                    if let Some(acceptor) = tls_acceptor {
                        let tls_stream = acceptor.accept(socket).await.with_context(|| {
                            format!("failed TLS handshake from {peer_address:?}")
                        })?;
                        Self::handle_client(tls_stream, peer_address, local_address, &params).await
                    } else {
                        Self::handle_client(socket, peer_address, local_address, &params).await
                    }
                }
                .await;

                if let Err(err) = result {
                    tracing::error!("proxy session error from {peer_address:?}: {err:#}");
                }
            });
        }
    }

    async fn handle_client<S>(
        stream: S,
        peer_address: SocketAddr,
        local_address: SocketAddr,
        params: &ProxyListenerParams,
    ) -> anyhow::Result<()>
    where
        S: AsyncReadAndWrite + Unpin + Send + 'static,
    {
        crate::proxy_handler::handle_proxy_client(
            stream,
            peer_address,
            local_address,
            params.timeout,
            params.use_splice,
            params.require_auth,
        )
        .await
    }
}

/// Connection metadata passed to the proxy_server_auth_rfc1929 callback
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct ConnMeta {
    pub peer_address: SocketAddr,
    pub local_address: SocketAddr,
}

impl IntoLua for ConnMeta {
    fn into_lua(self, lua: &Lua) -> mlua::Result<mlua::Value> {
        lua.to_value(&self)
    }
}

declare_event! {
    pub(crate) static CHECK_AUTH: Single(
        "proxy_server_auth_rfc1929",
        username: String,
        password: String,
        conn_meta: ConnMeta,
    ) -> SerdeWrappedValue<AuthKindResult>;
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "start_proxy_listener",
        lua.create_async_function(|lua, params: Value| async move {
            let params: ProxyListenerParams = from_lua_value(&lua, params)?;
            params.start().await.map_err(any_err)?;
            Ok(())
        })?,
    )?;

    Ok(())
}
