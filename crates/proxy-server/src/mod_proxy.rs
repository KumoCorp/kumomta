use config::{any_err, from_lua_value, get_or_create_module, CallbackSignature};
use data_loader::KeySource;
use kumo_server_runtime::spawn;
use mlua::{IntoLua, Lua, LuaSerdeExt, Value};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
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

    /// Disable splice(2) on Linux for proxied connections
    #[serde(default)]
    pub no_splice: bool,

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
            Self::accept_loop(listener, params, tls_acceptor).await
        })?;

        Ok(())
    }

    async fn accept_loop(
        listener: TcpListener,
        params: Arc<Self>,
        tls_acceptor: Option<TlsAcceptor>,
    ) {
        // Get the local address from the listener
        let local_address = match listener.local_addr() {
            Ok(addr) => addr,
            Err(err) => {
                tracing::error!("failed to get local address: {err:#}");
                return;
            }
        };

        loop {
            let (socket, peer_address) = match listener.accept().await {
                Ok(tuple) => tuple,
                Err(err) => {
                    tracing::error!("accept failed: {err:#}");
                    return;
                }
            };

            let params = params.clone();
            let tls_acceptor = tls_acceptor.clone();

            tokio::spawn(async move {
                let result = if let Some(acceptor) = tls_acceptor {
                    match acceptor.accept(socket).await {
                        Ok(tls_stream) => {
                            Self::handle_client(tls_stream, peer_address, local_address, &params)
                                .await
                        }
                        Err(err) => {
                            tracing::debug!("TLS handshake failed from {peer_address:?}: {err:#}");
                            return;
                        }
                    }
                } else {
                    Self::handle_client(socket, peer_address, local_address, &params).await
                };

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
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        crate::proxy_handler::handle_proxy_client(
            stream,
            peer_address,
            local_address,
            params.timeout,
            params.no_splice,
            params.require_auth,
        )
        .await
    }
}

/// Connection metadata passed to the proxy_server_auth_1929 callback
#[derive(Clone, Debug, serde::Serialize)]
pub struct ConnMeta {
    pub peer_address: String,
    pub local_address: String,
}

impl IntoLua for ConnMeta {
    fn into_lua(self, lua: &Lua) -> mlua::Result<mlua::Value> {
        lua.to_value(&self)
    }
}

/// Validates credentials via the proxy_server_auth_1929 Lua callback.
/// The callback receives (username, password, conn_meta) where conn_meta is a table
/// containing peer_address and local_address.
pub async fn authenticate_user(
    username: String,
    password: String,
    peer_address: SocketAddr,
    local_address: SocketAddr,
) -> anyhow::Result<bool> {
    let mut config = config::load_config().await?;

    let conn_meta = ConnMeta {
        peer_address: peer_address.to_string(),
        local_address: local_address.to_string(),
    };

    // Use tuple (username, password, conn_meta) for the callback signature
    let sig = CallbackSignature::<(String, String, ConnMeta), bool>::new("proxy_server_auth_1929");

    let result = config
        .async_call_callback(&sig, (username, password, conn_meta))
        .await?;
    config.put();

    Ok(result)
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
