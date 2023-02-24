use anyhow::Context;
use axum::extract::{FromRequestParts, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{async_trait, Router};
use axum_server::tls_rustls::RustlsConfig;
use cidr::IpCidr;
use config::load_config;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize, Clone, Debug)]
pub struct HttpListenerParams {
    #[serde(default = "HttpListenerParams::default_hostname")]
    pub hostname: String,

    #[serde(default = "HttpListenerParams::default_listen")]
    pub listen: String,

    #[serde(default)]
    pub use_tls: bool,

    #[serde(default)]
    pub tls_certificate: Option<PathBuf>,
    #[serde(default)]
    pub tls_private_key: Option<PathBuf>,

    #[serde(default = "HttpListenerParams::default_trusted_hosts")]
    pub trusted_hosts: Vec<IpCidr>,
}

#[derive(Clone)]
struct AppState {
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
            let config = self.tls_config()?;
            tracing::debug!("https listener on {addr:?}");
            let server = axum_server::from_tcp_rustls(socket, config);
            tokio::spawn(async move {
                server
                    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
            });
        } else {
            tracing::debug!("http listener on {addr:?}");
            let server = axum_server::from_tcp(socket);
            tokio::spawn(async move {
                server
                    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
            });
        }
        Ok(())
    }

    fn tls_config(&self) -> anyhow::Result<RustlsConfig> {
        let config = crate::tls_helpers::make_server_config(
            &self.hostname,
            &self.tls_private_key,
            &self.tls_certificate,
        )?;
        Ok(RustlsConfig::from_config(config))
    }
}

struct AppError(anyhow::Error);

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

/// Represents some authenticated identity.
/// Use this as an extractor parameter when you need to reference
/// that identity in the handler.
#[derive(Debug, Clone)]
pub enum AuthKind {
    TrustedIp(IpAddr),
    Basic {
        user: String,
        password: Option<String>,
    },
    Bearer {
        token: String,
    },
}

impl AuthKind {
    pub fn from_header(authorization: &str) -> Option<Self> {
        let (kind, contents) = authorization.split_once(' ')?;
        match kind {
            "Basic" => {
                let decoded = base64::decode(contents).ok()?;
                let decoded = String::from_utf8(decoded).ok()?;
                let (user, password) = if let Some((id, password)) = decoded.split_once(':') {
                    (id.to_string(), Some(password.to_string()))
                } else {
                    (decoded.to_string(), None)
                };
                Some(Self::Basic { user, password })
            }
            "Bearer" => Some(Self::Bearer {
                token: contents.to_string(),
            }),
            _ => None,
        }
    }

    async fn validate_impl(&self) -> anyhow::Result<bool> {
        let mut config = load_config().await?;
        match self {
            Self::TrustedIp(_) => Ok(true),
            Self::Basic { user, password } => Ok(config
                .async_call_callback(
                    "http_server_validate_auth_basic",
                    (user.to_string(), password.clone()),
                )
                .await?),
            Self::Bearer { token } => Ok(config
                .async_call_callback("http_server_validate_auth_bearer", token.to_string())
                .await?),
        }
    }

    pub async fn validate(&self) -> anyhow::Result<bool> {
        let kind = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        crate::runtime::Runtime::run(move || {
            tokio::task::spawn_local(async move { tx.send(kind.validate_impl().await) });
        })
        .await?;
        rx.await?
    }
}

async fn auth_middleware<B>(
    State(state): State<AppState>,
    mut request: Request<B>,
    next: Next<B>,
) -> Response {
    if let Some(remote_addr) = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0)
    {
        let ip = remote_addr.ip();
        if state.is_trusted_host(ip) {
            request.extensions_mut().insert(AuthKind::TrustedIp(ip));
            return next.run(request).await;
        }
    }

    // Get authorization header
    match request.headers().get(axum::http::header::AUTHORIZATION) {
        None => (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response(),
        Some(authorization) => match authorization.to_str() {
            Err(_) => (StatusCode::BAD_REQUEST, "Malformed Authorization header").into_response(),
            Ok(authorization) => match AuthKind::from_header(authorization) {
                None => (
                    StatusCode::BAD_REQUEST,
                    "Malformed or unsupported Authorization header",
                )
                    .into_response(),
                Some(kind) => match kind.validate().await {
                    Ok(true) => {
                        // Store the authentication inform for later retrieval
                        request.extensions_mut().insert(kind);
                        next.run(request).await
                    }
                    Ok(false) => {
                        (StatusCode::UNAUTHORIZED, "Invalid Authorization").into_response()
                    }
                    Err(err) => {
                        tracing::error!("Error validating {kind:?}: {err:#}");
                        (StatusCode::INTERNAL_SERVER_ERROR, "try again later").into_response()
                    }
                },
            },
        },
    }
}

#[async_trait]
impl<B> FromRequestParts<B> for AuthKind
where
    B: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &B,
    ) -> Result<Self, Self::Rejection> {
        let kind = parts
            .extensions
            .get::<AuthKind>()
            .ok_or((StatusCode::UNAUTHORIZED, "Unauthorized"))?;

        Ok(kind.clone())
    }
}

/// Use this type as an extractor parameter when the handler must
/// only be accessible to trusted IP addresses
pub struct TrustedIpRequired;

#[async_trait]
impl<B> FromRequestParts<B> for TrustedIpRequired
where
    B: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &B,
    ) -> Result<Self, Self::Rejection> {
        let kind = parts
            .extensions
            .get::<AuthKind>()
            .ok_or((StatusCode::UNAUTHORIZED, "Unauthorized"))?;

        match kind {
            AuthKind::TrustedIp(_) => Ok(TrustedIpRequired),
            _ => Err((StatusCode::UNAUTHORIZED, "Trusted IP required")),
        }
    }
}

async fn test_wat(auth: AuthKind) -> Result<String, AppError> {
    Ok(format!("Hello. auth={auth:?}"))
}
