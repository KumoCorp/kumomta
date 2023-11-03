use crate::http_server::AppState;
use axum::async_trait;
use axum::extract::{FromRequestParts, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use config::{load_config, CallbackSignature};
use kumo_server_runtime::rt_spawn;
use std::net::{IpAddr, SocketAddr};

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
            Self::Basic { user, password } => {
                let sig = CallbackSignature::<(String, Option<String>), bool>::new(
                    "http_server_validate_auth_basic",
                );
                Ok(config
                    .async_call_callback(&sig, (user.to_string(), password.clone()))
                    .await?)
            }
            Self::Bearer { token } => {
                let sig =
                    CallbackSignature::<String, bool>::new("http_server_validate_auth_bearer");
                Ok(config.async_call_callback(&sig, token.to_string()).await?)
            }
        }
    }

    pub async fn validate(&self) -> anyhow::Result<bool> {
        let kind = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        rt_spawn(format!("http auth validate {kind:?}"), move || {
            Ok(async move { tx.send(kind.validate_impl().await) })
        })
        .await?;
        rx.await?
    }

    pub fn summarize(&self) -> String {
        match self {
            Self::TrustedIp(addr) => addr.to_string(),
            Self::Basic { user, .. } => user.to_string(),
            Self::Bearer { .. } => "Bearer".to_string(),
        }
    }
}

pub async fn auth_middleware<B>(
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
