use crate::authn_authz::{
    ACLQueryDisposition, AccessControlList, AuthInfo, Identity, IdentityContext, Resource,
};
use crate::http_server::AppState;
use async_trait::async_trait;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use config::{load_config, CallbackSignature};
use std::net::{IpAddr, SocketAddr};
use tokio::time::{Duration, Instant};

lruttl::declare_cache! {
/// Caches the results of the http server auth validation by auth credential
static AUTH_CACHE: LruCacheWithTtl<AuthKind, Result<bool, String>>::new("http_server_auth", 128);
}

/// Represents some authenticated identity.
/// Use this as an extractor parameter when you need to reference
/// that identity in the handler.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
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
                let decoded = data_encoding::BASE64.decode(contents.as_bytes()).ok()?;
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

    async fn check_authentication_impl(&self) -> anyhow::Result<bool> {
        let mut config = load_config().await?;
        let result = match self {
            Self::TrustedIp(_) => true,
            Self::Basic { user, password } => {
                let sig = CallbackSignature::<(String, Option<String>), bool>::new(
                    "http_server_validate_auth_basic",
                );
                config
                    .async_call_callback(&sig, (user.to_string(), password.clone()))
                    .await?
            }
            Self::Bearer { token } => {
                let sig =
                    CallbackSignature::<String, bool>::new("http_server_validate_auth_bearer");
                config.async_call_callback(&sig, token.to_string()).await?
            }
        };
        config.put();
        Ok(result)
    }

    async fn lookup_cache(&self) -> Option<Result<bool, String>> {
        AUTH_CACHE.get(self)
    }

    pub async fn check_authentication(&self) -> anyhow::Result<bool> {
        match self.lookup_cache().await {
            Some(res) => res.map_err(|err| anyhow::anyhow!("{err}")),
            None => {
                let res = self
                    .check_authentication_impl()
                    .await
                    .map_err(|err| format!("{err:#}"));

                let res = AUTH_CACHE
                    .insert(self.clone(), res, Instant::now() + Duration::from_secs(60))
                    .await;

                res.map_err(|err| anyhow::anyhow!("{err}"))
            }
        }
    }

    pub fn summarize(&self) -> String {
        match self {
            Self::TrustedIp(addr) => addr.to_string(),
            Self::Basic { user, .. } => user.to_string(),
            Self::Bearer { .. } => "Bearer".to_string(),
        }
    }
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let mut auth_info = AuthInfo::default();
    let mut auth_kind = None;

    // Gather peer address info
    if let Some(remote_addr) = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0)
    {
        let ip = remote_addr.ip();

        // This is the authentic (as far as we're able to tell)
        // peer address, so record that as an identity.
        // There is no implicit trust associated with this fact.
        auth_info.set_peer_address(Some(ip));

        // If it is marked as trusted, update the auth kind state,
        // and populate an appropriate group that can later be
        // referenced in an ACL
        if state.is_trusted_host(ip) {
            auth_kind.replace(AuthKind::TrustedIp(ip));
            auth_info.add_group("kumomta:http-listener-trusted-ip");
        }
    }

    // Get authorization header
    if let Some(authorization) = request.headers().get(axum::http::header::AUTHORIZATION) {
        match authorization.to_str() {
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Malformed Authorization header").into_response()
            }
            Ok(authorization) => match AuthKind::from_header(authorization) {
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        "Malformed or unsupported Authorization header",
                    )
                        .into_response()
                }
                Some(kind) => match kind.check_authentication().await {
                    Ok(true) => {
                        // Store the authentication info for later retrieval
                        if let AuthKind::Basic { user, .. } = &kind {
                            auth_info.add_identity(Identity {
                                identity: user.to_string(),
                                context: IdentityContext::HttpBasicAuth,
                            });
                        }
                        auth_kind.replace(kind);
                    }
                    Ok(false) => {
                        return (StatusCode::UNAUTHORIZED, "Authentication Failed").into_response()
                    }
                    Err(err) => {
                        tracing::error!("Error validating {kind:?}: {err:#}");
                        return (StatusCode::INTERNAL_SERVER_ERROR, "try again later")
                            .into_response();
                    }
                },
            },
        }
    }

    // Populate authentication information into the request state
    if let Some(kind) = auth_kind.take() {
        request.extensions_mut().insert(kind);
    }
    request.extensions_mut().insert(auth_info.clone());

    // Check for authorization based on the URI + method
    match HttpEndpointResource::new(state.local_addr, request.uri()) {
        Ok(mut resource) => {
            let resource_id = resource.ident.to_string();
            let method = request.method().to_string();
            match AccessControlList::query_resource_access(&mut resource, &auth_info, &method).await
            {
                Ok(result) => match result {
                    ACLQueryDisposition::Allow { .. } => {}
                    ACLQueryDisposition::Deny { .. } => {
                        // In the response "denied GET on /something" means that
                        // there was an explicit Deny
                        return (
                            StatusCode::UNAUTHORIZED,
                            format!("{auth_info} denied {method} on {resource_id}"),
                        )
                            .into_response();
                    }
                    ACLQueryDisposition::DenyByDefault => {
                        // In the response "not permitted GET on /something" means
                        // that there was no explicit rule either way, and thus
                        // access was not permitted, but was also not explicitly
                        // denied.
                        return (
                            StatusCode::UNAUTHORIZED,
                            format!("{auth_info} not permitted {method} on {resource_id}"),
                        )
                            .into_response();
                    }
                },
                Err(err) => {
                    tracing::error!("Error querying ACL: {err:#}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "try again later").into_response();
                }
            }
        }
        Err(err) => {
            tracing::error!("Error building HttpEndpointResource: {err:#}");
            return (StatusCode::BAD_REQUEST, "malformed URI?").into_response();
        }
    }

    // Now allow the request to run
    next.run(request).await
}

#[derive(Clone)]
pub struct HttpEndpointResource {
    ident: String,
    iter: std::vec::IntoIter<String>,
}

/// Basic defense against a potentially abusive network client
const MAX_ACL_PATH_LEN: usize = 256;
const MAX_ACL_PATH_COMPONENTS: usize = 10;

impl HttpEndpointResource {
    pub fn new(local_addr: SocketAddr, uri: &Uri) -> anyhow::Result<Self> {
        let mut path: String = uri.path().to_string();
        path.truncate(MAX_ACL_PATH_LEN);
        let mut path_components: Vec<_> =
            path[1..].splitn(MAX_ACL_PATH_COMPONENTS + 1, '/').collect();
        path_components.truncate(MAX_ACL_PATH_COMPONENTS);

        let mut resources_with_host_and_port = vec![];
        let mut resources_with_path_only = vec![];

        while !path_components.is_empty() {
            let path = path_components.join("/");

            resources_with_host_and_port.push(format!("http_listener/{local_addr}/{path}"));
            resources_with_path_only.push(format!("http_listener/*/{path}"));

            path_components.pop();
        }
        resources_with_host_and_port.push(format!("http_listener/{local_addr}"));

        let mut resources = resources_with_host_and_port;
        resources.append(&mut resources_with_path_only);
        resources.push("http_listener".to_string());

        let ident = resources[0].clone();

        Ok(Self {
            ident,
            iter: resources.into_iter(),
        })
    }
}

#[async_trait]
impl Resource for HttpEndpointResource {
    fn resource_id(&self) -> &str {
        &self.ident
    }

    async fn next_resource_id(&mut self) -> Option<String> {
        self.iter.next()
    }
}

#[cfg(test)]
#[test]
fn test_http_endpoint_resource_expansion() {
    let res = HttpEndpointResource::new(
        "127.0.0.1:8080".parse().unwrap(),
        &Uri::from_static("https://user:pass@example.com:8080/foo/bar/baz"),
    )
    .unwrap();

    assert_eq!(
        res.iter.collect::<Vec<String>>(),
        [
            "http_listener/127.0.0.1:8080/foo/bar/baz",
            "http_listener/127.0.0.1:8080/foo/bar",
            "http_listener/127.0.0.1:8080/foo",
            "http_listener/127.0.0.1:8080",
            "http_listener/*/foo/bar/baz",
            "http_listener/*/foo/bar",
            "http_listener/*/foo",
            "http_listener"
        ]
        .into_iter()
        .map(Into::into)
        .collect::<Vec<String>>(),
    );
}

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
