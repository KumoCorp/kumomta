use crate::metrics::{tls_handshake_failures_for_listener, ProxySessionMetrics};
use anyhow::Context;
use config::{any_err, declare_event, get_or_create_module, SerdeWrappedValue};
use data_loader::KeySource;
use kumo_server_common::http_server::auth::AuthKindResult;
use kumo_server_common::http_server::{HttpListenerParams, RouterAndDocs};
use kumo_server_common::router_with_docs;
use kumo_server_runtime::{accept_error_pause, spawn};
use kumo_tls_helper::AsyncReadAndWrite;
use mlua::{IntoLua, Lua, LuaSerdeExt};
use serde::Deserialize;
use socket2::TcpKeepalive;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;
use utoipa::OpenApi;

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

    /// Maximum number of concurrent client connections. Connections accepted
    /// beyond this are closed immediately to bound file-descriptor and memory
    /// use.
    #[serde(default = "ProxyListenerParams::default_max_connections")]
    pub max_connections: usize,

    /// TCP keepalive parameters applied to inbound and outbound proxy
    /// sockets.  Controls whether and how often the kernel probes an idle
    /// connection so that unresponsive peers are detected and closed.
    #[serde(default)]
    pub tcp_keepalive: TcpKeepaliveParams,
}

fn default_true() -> bool {
    true
}

/// Tunable TCP keepalive parameters.
///
/// `time` is the idle period before the first probe is sent. On Linux,
/// `interval` and `retries` control the spacing and count of probes after
/// the first; on other platforms only `time` is configurable.
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct TcpKeepaliveParams {
    /// Whether TCP keepalive is enabled. Defaults to true.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Idle time before the first keepalive probe is sent.
    #[serde(default = "TcpKeepaliveParams::default_time", with = "duration_serde")]
    pub time: Duration,

    /// Interval between keepalive probes after the first (Linux only).
    #[serde(
        default = "TcpKeepaliveParams::default_interval",
        with = "duration_serde"
    )]
    pub interval: Duration,

    /// Number of unanswered probes before the connection is considered
    /// dead (Linux only).
    #[serde(default = "TcpKeepaliveParams::default_retries")]
    pub retries: u32,
}

impl Default for TcpKeepaliveParams {
    fn default() -> Self {
        Self {
            enabled: true,
            time: Self::default_time(),
            interval: Self::default_interval(),
            retries: Self::default_retries(),
        }
    }
}

impl TcpKeepaliveParams {
    fn default_time() -> Duration {
        Duration::from_secs(300)
    }

    fn default_interval() -> Duration {
        Duration::from_secs(30)
    }

    fn default_retries() -> u32 {
        3
    }

    /// Build a `socket2::TcpKeepalive` matching these parameters, or
    /// `None` if keepalive is disabled.
    pub fn build(&self) -> Option<TcpKeepalive> {
        if !self.enabled {
            return None;
        }
        let keepalive = TcpKeepalive::new()
            .with_time(self.time)
            .with_interval(self.interval)
            .with_retries(self.retries);

        Some(keepalive)
    }
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

    fn default_max_connections() -> usize {
        32 * 1024
    }

    /// Start the proxy listener
    pub async fn start(self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(&self.listen).await?;
        let addr = listener.local_addr()?;
        let keepalive = Arc::new(self.tcp_keepalive.build());

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

        let connection_limiter = Arc::new(Semaphore::new(self.max_connections));
        let params = Arc::new(self);

        spawn(format!("proxy listener {addr:?}"), async move {
            Self::accept_loop(
                listener,
                params,
                tls_acceptor,
                keepalive,
                connection_limiter,
            )
            .await
        })?;

        Ok(())
    }

    async fn accept_loop(
        listener: TcpListener,
        params: Arc<Self>,
        tls_acceptor: Option<TlsAcceptor>,
        keepalive: Arc<Option<TcpKeepalive>>,
        connection_limiter: Arc<Semaphore>,
    ) {
        let local_address = match listener.local_addr() {
            Ok(addr) => addr,
            Err(err) => {
                tracing::error!("failed to get local address for proxy listener: {err:#}");
                return;
            }
        };

        let denied = crate::metrics::connections_denied_for_listener(local_address);
        let accept_errors = crate::metrics::accept_errors_for_listener(local_address);

        loop {
            let (socket, peer_address) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(err) => {
                    // Keep looping instead of returning: exiting here would
                    // silently stop serving this port while the process stays
                    // alive, which looks healthy to any supervisor watching it.
                    match accept_error_pause(&err) {
                        Some(pause) => {
                            // Non-connection-level errors such as EMFILE are
                            // rare and significant: count them and log at error
                            // level.
                            accept_errors.inc();
                            tracing::error!(
                                "proxy listener on {local_address:?} accept failed: {err:#}"
                            );
                            // Pause before the next accept() to avoid spinning
                            // the loop while the condition persists.
                            tokio::time::sleep(pause).await;
                        }
                        None => {
                            // A peer that reset between the kernel queuing the
                            // connection and our accepting it. Any peer can
                            // trigger this on demand. Report it only at debug
                            // level to deny a hostile peer a way to flood the
                            // logs.
                            tracing::debug!("proxy listener on {local_address:?} accept: {err:#}");
                        }
                    }
                    continue;
                }
            };

            let Ok(permit) = connection_limiter.clone().try_acquire_owned() else {
                // Count over-limit connections rather than logging each one: a
                // peer can hit the limit on every attempt, and a per-connection
                // log line would let it flood the logs. The counter still gives
                // an operator a signal to act on.
                denied.inc();
                // Close to prevent fd and memory use from growing unbounded.
                continue;
            };

            if let Err(err) =
                crate::proxy_handler::set_proxy_tcp_keepalive(&socket, keepalive.as_ref().as_ref())
            {
                tracing::error!(
                    "failed to enable TCP keepalive for client {peer_address:?}: {err:#}"
                );
                continue;
            }

            let params = params.clone();
            let tls_acceptor = tls_acceptor.clone();
            let keepalive = keepalive.clone();

            tokio::spawn(async move {
                let _permit = permit;
                let result: anyhow::Result<()> = async {
                    if let Some(acceptor) = tls_acceptor {
                        let tls_stream = match acceptor.accept(socket).await {
                            Ok(stream) => stream,
                            Err(err) => {
                                // Track TLS handshake failure
                                tls_handshake_failures_for_listener(local_address).inc();
                                return Err(err).with_context(|| {
                                    format!("failed TLS handshake from {peer_address:?}")
                                });
                            }
                        };
                        Self::handle_client(
                            tls_stream,
                            peer_address,
                            local_address,
                            &params,
                            keepalive,
                        )
                        .await
                    } else {
                        Self::handle_client(socket, peer_address, local_address, &params, keepalive)
                            .await
                    }
                }
                .await;

                if let Err(err) = result {
                    tracing::error!("proxy session error from {peer_address:?}: {err:#}");
                }
            });
        }
    }

    /// Handle a single client connection with centralized metrics tracking.
    ///
    /// This function is responsible for:
    /// - Tracking connection acceptance
    /// - Managing the ProxySessionMetrics lifecycle (RAII for active connections)
    /// - Recording success/failure and bytes transferred
    async fn handle_client<S>(
        stream: S,
        peer_address: SocketAddr,
        local_address: SocketAddr,
        params: &ProxyListenerParams,
        keepalive: Arc<Option<TcpKeepalive>>,
    ) -> anyhow::Result<()>
    where
        S: AsyncReadAndWrite + Unpin + Send + 'static,
    {
        // Create session metrics (increments active connections gauge via RAII)
        let session_metrics = ProxySessionMetrics::new(local_address);

        // Perform the actual proxy work
        let result = crate::proxy_handler::handle_proxy_client(
            stream,
            peer_address,
            local_address,
            params.timeout,
            params.use_splice,
            params.require_auth,
            keepalive,
        )
        .await;

        // Update metrics based on result
        match result {
            Ok(session_result) => {
                session_metrics.record_bytes(
                    session_result.bytes_to_remote,
                    session_result.bytes_to_client,
                );
                session_metrics.mark_completed();
                Ok(())
            }
            Err(err) => {
                session_metrics.mark_failed();
                Err(err)
            }
        }
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

/// Create the router for the proxy HTTP endpoint
pub fn make_router() -> RouterAndDocs {
    router_with_docs! {
       title = "proxy-server",
       handlers = [
          proxy_status
       ]
    }
}

/// Simple health check endpoint for the proxy.
/// Returns basic status information.
#[utoipa::path(
    get,
    tag = "status",
    path = "/proxy/status",
    responses(
        (status = 200, description = "Proxy is healthy", body = String)
    ),
)]
async fn proxy_status() -> &'static str {
    "KumoProxy OK"
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let proxy_mod = get_or_create_module(lua, "proxy")?;
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    async fn start_proxy_listener(
        _lua: Lua,
        params: SerdeWrappedValue<ProxyListenerParams>,
    ) -> mlua::Result<()> {
        if !config::is_validating() {
            params.0.start().await.map_err(any_err)?;
        }
        Ok(())
    }

    // Briefly was available in the kumo namespace, we're doing
    // a little backwards compat here in case folks were running
    // this somewhere important from dev builds.
    kumo_mod.set(
        "start_proxy_listener",
        lua.create_async_function(start_proxy_listener)?,
    )?;

    proxy_mod.set(
        "start_proxy_listener",
        lua.create_async_function(start_proxy_listener)?,
    )?;

    proxy_mod.set(
        "start_http_listener",
        lua.create_async_function(
            |_lua, params: SerdeWrappedValue<HttpListenerParams>| async move {
                if !config::is_validating() {
                    params.0.start(make_router(), None).await.map_err(any_err)?;
                }
                Ok(())
            },
        )?,
    )?;

    Ok(())
}
