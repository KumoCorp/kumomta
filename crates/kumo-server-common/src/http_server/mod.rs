use crate::diagnostic_logging::set_diagnostic_log_filter;
use anyhow::Context;
use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use cidr_map::{AnyIpCidr, CidrSet};
use data_loader::KeySource;
use kumo_server_runtime::spawn;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::str::FromStr;
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::OpenApi;
use utoipa_rapidoc::RapiDoc;
// Avoid referencing api types as crate::name in the utoipa macros,
// otherwise it generates namespaced names in the openapi.json, which
// in turn require annotating each and every struct with the namespace
// in order for the document to be valid.
use kumo_api_types::*;

pub mod auth;

use auth::*;

#[derive(OpenApi)]
#[openapi(
    info(license(name = "Apache-2.0")),
    paths(set_diagnostic_log_filter_v1),
    // Indicate that all paths can accept http basic auth.
    // the "basic_auth" name corresponds with the scheme
    // defined by the OptionalAuth addon defined below
    security(
        ("basic_auth" = [""])
    ),
    components(schemas(SetDiagnosticFilterRequest)),
    modifiers(&OptionalAuth),
)]
struct ApiDoc;

struct OptionalAuth;

impl utoipa::Modify for OptionalAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .as_mut()
            .expect("always set because we always have components above");
        // Define basic_auth as http basic auth
        components.add_security_scheme(
            "basic_auth",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Basic)),
        );
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
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
    pub trusted_hosts: CidrSet,
}

pub struct RouterAndDocs {
    pub router: Router,
    pub docs: utoipa::openapi::OpenApi,
}

impl RouterAndDocs {
    pub fn make_docs(&self) -> utoipa::openapi::OpenApi {
        let mut api_docs = ApiDoc::openapi();
        api_docs.info.title = self.docs.info.title.to_string();
        api_docs.merge(self.docs.clone());
        api_docs.info.version = version_info::kumo_version().to_string();
        api_docs.info.license = Some(
            utoipa::openapi::LicenseBuilder::new()
                .name("Apache-2.0")
                .build(),
        );

        api_docs
    }
}

#[derive(Clone)]
pub struct AppState {
    trusted_hosts: Arc<CidrSet>,
}

impl AppState {
    pub fn is_trusted_host(&self, addr: IpAddr) -> bool {
        self.trusted_hosts.contains(addr)
    }
}

impl HttpListenerParams {
    fn default_listen() -> String {
        "127.0.0.1:8000".to_string()
    }

    fn default_trusted_hosts() -> CidrSet {
        [
            AnyIpCidr::from_str("127.0.0.1").unwrap(),
            AnyIpCidr::from_str("::1").unwrap(),
        ]
        .into()
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
    pub async fn start(self, router_and_docs: RouterAndDocs) -> anyhow::Result<()> {
        let api_docs = router_and_docs.make_docs();

        let app = router_and_docs
            .router
            .merge(RapiDoc::with_openapi("/api-docs/openapi.json", api_docs).path("/rapidoc"))
            .route(
                "/api/admin/set_diagnostic_log_filter/v1",
                post(set_diagnostic_log_filter_v1),
            )
            .route("/metrics", get(report_metrics))
            .route("/metrics.json", get(report_metrics_json))
            // Require that all requests be authenticated as either coming
            // from a trusted IP address, or with an authorization header
            .route_layer(axum::middleware::from_fn_with_state(
                AppState {
                    trusted_hosts: Arc::new(self.trusted_hosts.clone()),
                },
                auth_middleware,
            ))
            .layer(TraceLayer::new_for_http());
        let socket = TcpListener::bind(&self.listen)
            .with_context(|| format!("listen on {}", self.listen))?;
        let addr = socket.local_addr()?;

        if self.use_tls {
            let config = self.tls_config().await?;
            tracing::info!("https listener on {addr:?}");
            let server = axum_server::from_tcp_rustls(socket, config);
            spawn(format!("https {addr:?}"), async move {
                server
                    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
            })?;
        } else {
            tracing::info!("http listener on {addr:?}");
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

#[derive(Debug)]
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

async fn report_metrics_json(_: TrustedIpRequired) -> Result<Json<serde_json::Value>, AppError> {
    use prometheus::proto::MetricType;
    use serde_json::{json, Map, Number, Value};

    let mut result = Map::new();

    let metrics = prometheus::default_registry().gather();
    for mf in metrics {
        let name = mf.get_name();
        let help = mf.get_help();

        let mut family = Map::new();
        let metric_type = mf.get_field_type();
        family.insert(
            "type".to_string(),
            format!("{metric_type:?}").to_lowercase().into(),
        );
        if !help.is_empty() {
            family.insert("help".to_string(), help.into());
        }

        let mut value = Value::Null;

        fn apply_to_value(
            value: &mut Value,
            this_value: Value,
            label: &[prometheus::proto::LabelPair],
        ) {
            if label.is_empty() {
                *value = this_value;
            } else if label.len() == 1 {
                let only_pair = &label[0];
                if value.is_null() {
                    let mut map = Map::new();
                    map.insert(only_pair.get_name().to_string(), json!({}));
                    *value = Value::Object(map);
                }
                let by_label = value
                    .as_object_mut()
                    .unwrap()
                    .entry(only_pair.get_name().to_string())
                    .or_insert_with(|| json!({}));
                by_label
                    .as_object_mut()
                    .unwrap()
                    .insert(only_pair.get_value().to_string(), this_value);
            } else {
                if value.is_null() {
                    *value = json!([]);
                }
                let mut pairs = Map::new();
                for p in label {
                    pairs.insert(p.get_name().to_string(), p.get_value().to_string().into());
                }
                pairs.insert("@".to_string(), this_value);
                value.as_array_mut().unwrap().push(Value::Object(pairs));
            }
        }

        for mc in mf.get_metric() {
            let label = mc.get_label();

            match metric_type {
                MetricType::COUNTER => {
                    let this_value = Value::Number(
                        Number::from_f64(mc.get_counter().get_value()).unwrap_or_else(|| 0.into()),
                    );

                    apply_to_value(&mut value, this_value, label);
                }
                MetricType::GAUGE => {
                    let this_value = Value::Number(
                        Number::from_f64(mc.get_gauge().get_value()).unwrap_or_else(|| 0.into()),
                    );

                    apply_to_value(&mut value, this_value, label);
                }
                _ => {
                    // Other types are currently not implemented
                    // as we don't currently export any other type
                }
            }
        }

        family.insert("value".to_string(), value);

        result.insert(name.to_string(), Value::Object(family));
    }

    Ok(Json(Value::Object(result)))
}

/// Changes the diagnostic log filter dynamically.
/// See <https://docs.kumomta.com/reference/kumo/set_diagnostic_log_filter/>
/// for more information on diagnostic log filters.
#[utoipa::path(
    post,
    tag="logging",
    path="/api/admin/set_diagnostic_log_filter/v1",
    responses(
        (status = 200, description = "Diagnostic level set successfully")
    ),
)]
async fn set_diagnostic_log_filter_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(request): Json<SetDiagnosticFilterRequest>,
) -> Result<(), AppError> {
    set_diagnostic_log_filter(&request.filter)?;
    Ok(())
}
