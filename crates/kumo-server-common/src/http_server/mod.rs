use crate::diagnostic_logging::set_diagnostic_log_filter;
use anyhow::Context;
use axum::extract::{DefaultBodyLimit, Json, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use axum_streams::{HttpHeaderValue, StreamBodyAsOptions};
use cidr_map::CidrSet;
use data_loader::KeySource;
use kumo_server_memory::{get_usage_and_limit, tracking_stats, JemallocStats};
use kumo_server_runtime::spawn;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::decompression::RequestDecompressionLayer;
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
    paths(set_diagnostic_log_filter_v1, bump_config_epoch),
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
    pub request_body_limit: Option<usize>,

    #[serde(default)]
    pub tls_certificate: Option<KeySource>,
    #[serde(default)]
    pub tls_private_key: Option<KeySource>,

    #[serde(default = "CidrSet::default_trusted_hosts")]
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
    pub async fn start(
        self,
        router_and_docs: RouterAndDocs,
        runtime: Option<tokio::runtime::Handle>,
    ) -> anyhow::Result<()> {
        let api_docs = router_and_docs.make_docs();

        let compression_layer: CompressionLayer = CompressionLayer::new()
            .deflate(true)
            .gzip(true)
            .quality(tower_http::CompressionLevel::Fastest);
        let decompression_layer = RequestDecompressionLayer::new().deflate(true).gzip(true);
        let app = router_and_docs
            .router
            .layer(DefaultBodyLimit::max(
                self.request_body_limit.unwrap_or(2 * 1024 * 1024),
            ))
            .merge(RapiDoc::with_openapi("/api-docs/openapi.json", api_docs).path("/rapidoc"))
            .route(
                "/api/admin/set_diagnostic_log_filter/v1",
                post(set_diagnostic_log_filter_v1),
            )
            .route("/api/admin/bump-config-epoch", post(bump_config_epoch))
            .route("/api/admin/memory/stats", get(memory_stats))
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
            .layer(compression_layer)
            .layer(decompression_layer)
            .layer(TraceLayer::new_for_http());
        let socket = TcpListener::bind(&self.listen)
            .with_context(|| format!("listen on {}", self.listen))?;
        let addr = socket.local_addr()?;

        let make_service = app.into_make_service_with_connect_info::<SocketAddr>();

        // The logic below is a bit repeatey, but it is still fewer
        // lines of magic than it would be to factor out into a
        // generic function because of all of the trait bounds
        // that it would require.
        if self.use_tls {
            let config = self.tls_config().await?;
            tracing::info!("https listener on {addr:?}");
            let server = axum_server::from_tcp_rustls(socket, config);
            let serve = async move { server.serve(make_service).await };

            if let Some(runtime) = runtime {
                runtime.spawn(serve);
            } else {
                spawn(format!("https {addr:?}"), serve)?;
            }
        } else {
            tracing::info!("http listener on {addr:?}");
            let server = axum_server::from_tcp(socket);
            let serve = async move { server.serve(make_service).await };
            if let Some(runtime) = runtime {
                runtime.spawn(serve);
            } else {
                spawn(format!("http {addr:?}"), serve)?;
            }
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
pub struct AppError(pub anyhow::Error);

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

/// Allows the system operator to trigger a configuration epoch bump,
/// which causes various configs that are using the Epoch strategy to
/// be re-evaluated by triggering the appropriate callbacks.
#[utoipa::path(
    post,
    tag="config",
    path="/api/admin/bump-config-epoch",
    responses(
        (status=200, description = "bump successful")
    ),
)]
async fn bump_config_epoch(_: TrustedIpRequired) -> Result<(), AppError> {
    config::epoch::bump_current_epoch();
    Ok(())
}

/// Returns information about the system memory usage in an unstructured
/// human readable format.  The output is not machine parseable and may
/// change without notice between versions of kumomta.
#[utoipa::path(
    get,
    tag="memory",
    path="/api/admin/memory/stats",
    responses(
        (status=200, description = "stats were returned")
    ),
)]
async fn memory_stats(_: TrustedIpRequired) -> String {
    use kumo_server_memory::NumBytes;
    use std::fmt::Write;
    let mut result = String::new();

    let jstats = JemallocStats::collect();
    writeln!(result, "{jstats:#?}").ok();

    if let Ok((usage, limit)) = get_usage_and_limit() {
        writeln!(result, "RSS = {:?}", NumBytes::from(usage.bytes)).ok();
        writeln!(
            result,
            "soft limit = {:?}",
            limit.soft_limit.map(NumBytes::from)
        )
        .ok();
        writeln!(
            result,
            "hard limit = {:?}",
            limit.hard_limit.map(NumBytes::from)
        )
        .ok();
    }

    let mut stats = tracking_stats();
    writeln!(result, "live = {:?}", stats.live).ok();

    if stats.top_callstacks.is_empty() {
        write!(
            result,
            "\nuse kumo.enable_memory_callstack_tracking(true) to enable additional stats\n"
        )
        .ok();
    } else {
        writeln!(result, "small_threshold = {:?}", stats.small_threshold).ok();
        write!(result, "\ntop call stacks:\n").ok();
        for stack in &mut stats.top_callstacks {
            writeln!(
                result,
                "sampled every {} allocations, estimated {} allocations of {} total bytes",
                stack.stochastic_rate,
                stack.count * stack.stochastic_rate,
                stack.total_size * stack.stochastic_rate
            )
            .ok();
            write!(result, "{:?}\n\n", stack.bt).ok();
        }
    }

    result
}

#[derive(Deserialize)]
struct PrometheusMetricsParams {
    #[serde(default)]
    prefix: Option<String>,
}

async fn report_metrics(
    _: TrustedIpRequired,
    Query(params): Query<PrometheusMetricsParams>,
) -> impl IntoResponse {
    StreamBodyAsOptions::new()
        .content_type(HttpHeaderValue::from_static("text/plain; charset=utf-8"))
        .text(kumo_prometheus::registry::Registry::stream_text(
            params.prefix.clone(),
        ))
}

async fn report_metrics_json(_: TrustedIpRequired) -> impl IntoResponse {
    StreamBodyAsOptions::new()
        .content_type(HttpHeaderValue::from_static(
            "application/json; charset=utf-8",
        ))
        .text(kumo_prometheus::registry::Registry::stream_json())
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
