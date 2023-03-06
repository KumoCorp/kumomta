use crate::runtime::spawn;
use anyhow::Context;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use cidr::IpCidr;
use data_loader::KeySource;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::Arc;

pub mod auth;

pub mod admin_bounce_v1;
pub mod inject_v1;

use auth::*;

#[derive(Deserialize, Clone, Debug)]
pub struct HttpListenerParams {
    #[serde(default = "HttpListenerParams::default_hostname")]
    pub hostname: String,

    #[serde(default = "HttpListenerParams::default_listen")]
    pub listen: String,

    #[serde(default)]
    pub use_tls: bool,

    #[serde(default)]
    pub tls_certificate: Option<KeySource>,
    #[serde(default)]
    pub tls_private_key: Option<KeySource>,

    #[serde(default = "HttpListenerParams::default_trusted_hosts")]
    pub trusted_hosts: Vec<IpCidr>,
}

#[derive(Clone)]
pub struct AppState {
    trusted_hosts: Arc<Vec<IpCidr>>,
}

impl AppState {
    pub fn is_trusted_host(&self, addr: IpAddr) -> bool {
        for cidr in self.trusted_hosts.iter() {
            if cidr.contains(&addr) {
                return true;
            }
        }
        false
    }
}

impl HttpListenerParams {
    fn default_listen() -> String {
        "127.0.0.1:8000".to_string()
    }

    fn default_trusted_hosts() -> Vec<IpCidr> {
        vec![
            IpCidr::new("127.0.0.1".parse().unwrap(), 32).unwrap(),
            IpCidr::new("::1".parse().unwrap(), 128).unwrap(),
        ]
    }

    fn default_hostname() -> String {
        gethostname::gethostname()
            .to_str()
            .unwrap_or("localhost")
            .to_string()
    }

    // Note: it is possible to call
    // server.with_graceful_shutdown(ShutdownSubscription::get().shutting_down)
    // to have it listen for a shutdown request, but we're avoiding it:
    // the request is the start of a shutdown and we need to allow a grace
    // period for in-flight operations to complete.
    // Some of those may require call backs to the HTTP endpoint
    // if we're doing some kind of web hook like thing.
    // So, for now at least, we'll have to manually verify if
    // a request should proceed based on the results from the lifecycle
    // module.
    pub async fn start(self) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/metrics", get(report_metrics))
            .route("/api/inject/v1", post(inject_v1::inject_v1))
            .route("/api/admin/bounce/v1", post(admin_bounce_v1::bounce_v1))
            .route("/wat", get(test_wat))
            // Require that all requests be authenticated as either coming
            // from a trusted IP address, or with an authorization header
            .route_layer(axum::middleware::from_fn_with_state(
                AppState {
                    trusted_hosts: Arc::new(self.trusted_hosts.clone()),
                },
                auth_middleware,
            ));
        let addr: SocketAddr = self.listen.parse()?;
        let socket = TcpListener::bind(&self.listen)
            .with_context(|| format!("listen on {}", self.listen))?;

        if self.use_tls {
            let config = self.tls_config().await?;
            tracing::debug!("https listener on {addr:?}");
            let server = axum_server::from_tcp_rustls(socket, config);
            spawn(format!("https {addr:?}"), async move {
                server
                    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
            })?;
        } else {
            tracing::debug!("http listener on {addr:?}");
            let server = axum_server::from_tcp(socket);
            spawn(format!("http {addr:?}"), async move {
                server
                    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
            })?;
        }
        Ok(())
    }

    async fn tls_config(&self) -> anyhow::Result<RustlsConfig> {
        let config = crate::tls_helpers::make_server_config(
            &self.hostname,
            &self.tls_private_key,
            &self.tls_certificate,
        )
        .await?;
        Ok(RustlsConfig::from_config(config))
    }
}

pub struct AppError(anyhow::Error);

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Error: {:#}", self.0),
        )
            .into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, AppError>`. That way you don't need to do that manually.
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

async fn report_metrics(_: TrustedIpRequired) -> Result<String, AppError> {
    let report = prometheus::TextEncoder::new()
        .encode_to_string(&prometheus::default_registry().gather())?;
    Ok(report)
}

async fn test_wat(auth: AuthKind) -> Result<String, AppError> {
    Ok(format!("Hello. auth={auth:?}"))
}
