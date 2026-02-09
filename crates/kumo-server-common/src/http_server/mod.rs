use crate::diagnostic_logging::set_diagnostic_log_filter;
use anyhow::Context;
use axum::extract::{DefaultBodyLimit, Json, Query};
use axum::handler::Handler;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use axum_streams::{HttpHeaderValue, StreamBodyAsOptions};
use cidr_map::CidrSet;
use data_loader::KeySource;
use kumo_server_memory::{get_usage_and_limit, tracking_stats, JemallocStats};
use kumo_server_runtime::spawn;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr, TcpListener};
use tower_http::compression::CompressionLayer;
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::trace::TraceLayer;
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::openapi::PathItem;
use utoipa::OpenApi;
use utoipa_rapidoc::RapiDoc;
// Avoid referencing api types as crate::name in the utoipa macros,
// otherwise it generates namespaced names in the openapi.json, which
// in turn require annotating each and every struct with the namespace
// in order for the document to be valid.
use kumo_api_types::*;

pub mod auth;

use auth::*;

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

/// Encapsulates both an axum router and a set of OpenApi docs.
/// Use the router_with_docs! macro to create one of these.
pub struct RouterAndDocs {
    pub router: Router<AppState>,
    pub docs: utoipa::openapi::OpenApi,
}

impl RouterAndDocs {
    /// Helper to figure out what kind of handler method should be
    /// used to wrap the provided handler.
    /// It will examine the provided PathItem to figure out which
    /// operation was defined and use that.
    fn add_route<T, H: Handler<T, AppState>>(&mut self, path: &str, item: &PathItem, handler: H)
    where
        T: 'static,
        H: 'static,
    {
        let router = std::mem::take(&mut self.router);
        if item.get.is_some() {
            self.router = router.route(path, axum::routing::get(handler));
        } else if item.put.is_some() {
            self.router = router.route(path, axum::routing::put(handler));
        } else if item.post.is_some() {
            self.router = router.route(path, axum::routing::post(handler));
        } else if item.delete.is_some() {
            self.router = router.route(path, axum::routing::delete(handler));
        } else {
            panic!("unhandled path operation");
        }
    }

    /// Register a route based on the definition contained in the provided
    /// OpenApi docs. This will extract the path and correct operation
    /// type from the openapi docs and use that to register the handler,
    /// allowing the path and operation for the handler to be centrally
    /// defined in the `utoipa::path` macro annotation on the handler
    /// itself.
    ///
    /// You will not call this directly: it will be called via the
    /// `router_with_docs!` macro invocation.
    ///
    /// The OpenApi instance MUST have only a single path with
    /// a single operation defined within it.  This is upheld
    /// in the macro implementation.
    pub fn register<T, H: Handler<T, AppState>>(
        &mut self,
        api: utoipa::openapi::OpenApi,
        handler: H,
    ) where
        T: 'static,
        H: 'static,
    {
        if let Some((path, item)) = api.paths.paths.iter().next() {
            self.add_route(path, item, handler);
        } else {
            panic!("register didn't register any paths!");
        }

        self.docs.merge(api);
    }

    /// Create a new RouterAndDocs instance.
    /// You should trigger this via the `router_with_docs!` macro
    /// rather than using it directly.
    pub fn new(title: &str) -> Self {
        #[derive(OpenApi)]
        #[openapi(
                info(
                    license(name="Apache-2.0"),
                    version=version_info::kumo_version()
                ),
                // Indicate that all paths can accept http basic auth.
                // the "basic_auth" name corresponds with the scheme
                // defined by the OptionalAuth addon defined below
                security(
                    ("basic_auth" = [""])
                ),
                modifiers(&OptionalAuth),
            )]
        struct ApiDoc;
        let mut router = Self {
            docs: ApiDoc::openapi(),
            router: Router::new(),
        };

        router.docs.info.title = title.to_string();

        router.register_common_handlers();
        router
    }

    /// Register the default/common handlers.
    fn register_common_handlers(&mut self) {
        macro_rules! add_handlers {
            ($($handler:path $(,)?)*) => {
                $(
                {
                    // See the `router_with_docs!` definition below
                    // for an explanation of this `O` struct.
                    #[derive(OpenApi)]
                    #[openapi(paths($handler))]
                    struct O;

                    self.register(O::openapi(), $handler);
                }
                )*
            }
        }

        add_handlers!(
            bump_config_epoch,
            memory_stats,
            report_metrics,
            report_metrics_json,
            set_diagnostic_log_filter_v1,
        );
    }
}

/// Create a RouterAndDocs instance and register each
/// of the handlers into it.
///
/// Each handler must be an axum compatible handler
/// that has a `utoipa::path` annotation to define
/// its method and path.
#[macro_export]
macro_rules! router_with_docs {
    (title=$title:literal, handlers=[
     $($handler:path $(,)?  )*
    ]
    $(, layers=[
        $(
            $layer:expr $(,)?
        )*
    ])?

    ) => {
        {
            // Allow adding deprecated routes without
            // triggering a warning; the deprecation
            // status flows through to the docs
            #![allow(deprecated)]

            let mut router = RouterAndDocs::new($title);

            $(
                // Utoipa's path macro defines an auxilliary
                // path configuration struct at `__path_{ident}`.
                // We can't "know" that here as it is a funky
                // implementation detail, and rust provides no
                // way for us to magic up a reference to that
                // identifier.
                // Instead, we leverage the proc macro that
                // derives an OpenApi impl; its `paths` parameter
                // knows how to find the path configuration given
                // the bare/normal handler reference.
                // Armed with that OpenApi impl, we can then
                // merge it into our router via our register
                // method.
                {
                    #[derive(OpenApi)]
                    #[openapi(paths($handler))]
                    struct O;

                    router.register(O::openapi(), $handler);
                }
            )*

            $(
                $(
                    router.router = router.router.layer($layer);
                )*
            )?

            router
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    params: HttpListenerParams,
    local_addr: SocketAddr,
}

impl AppState {
    pub fn is_trusted_host(&self, addr: IpAddr) -> bool {
        self.params.trusted_hosts.contains(addr)
    }

    pub fn params(&self) -> &HttpListenerParams {
        &self.params
    }

    pub fn local_addr(&self) -> &SocketAddr {
        &self.local_addr
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
        let compression_layer: CompressionLayer = CompressionLayer::new()
            .deflate(true)
            .gzip(true)
            .quality(tower_http::CompressionLevel::Fastest);
        let decompression_layer = RequestDecompressionLayer::new().deflate(true).gzip(true);

        let socket = TcpListener::bind(&self.listen)
            .with_context(|| format!("listen on {}", self.listen))?;
        let addr = socket.local_addr()?;

        let app_state = AppState {
            params: self.clone(),
            local_addr: addr.clone(),
        };

        let app = router_and_docs
            .router
            .layer(DefaultBodyLimit::max(
                self.request_body_limit.unwrap_or(2 * 1024 * 1024),
            ))
            .merge(
                RapiDoc::with_openapi("/api-docs/openapi.json", router_and_docs.docs)
                    .path("/rapidoc"),
            )
            // Require that all requests be authenticated as either coming
            // from a trusted IP address, or with an authorization header
            .route_layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                auth_middleware,
            ))
            .layer(compression_layer)
            .layer(decompression_layer)
            .layer(TraceLayer::new_for_http())
            .layer(axum_client_ip::ClientIpSource::ConnectInfo.into_extension())
            .with_state(app_state);

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
            &None,
        )
        .await?;
        Ok(RustlsConfig::from_config(config))
    }
}

#[derive(Debug)]
pub struct AppError {
    pub err: anyhow::Error,
    pub code: StatusCode,
}

impl AppError {
    pub fn new(code: StatusCode, err: impl Into<String>) -> Self {
        let err: String = err.into();
        Self {
            err: anyhow::anyhow!(err),
            code,
        }
    }
}

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.code, format!("Error: {:#}", self.err)).into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, AppError>`. That way you don't need to do that manually.
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self {
            err: err.into(),
            code: StatusCode::INTERNAL_SERVER_ERROR,
        }
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
async fn bump_config_epoch() -> Result<(), AppError> {
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
async fn memory_stats() -> String {
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

/// Returns the current set of metrics in
/// [Prometheus Text Exposition Format](https://prometheus.io/docs/instrumenting/exposition_formats/).
///
/// !!! note
///     Metrics generally represent data at the current point in time,
///     to be consumed by an external system (such as Prometheus) which
///     then in turn can build time series data around those metrics.
///
///     In addition, in order to avoid unbounded RAM usage for systems
///     with many queues, a number of queue- or service-specific metrics
///     will be automatically pruned away when the corresponding queue
///     idles out for a period of time.
///
/// In the default configuration, access to this endpoint requires *Trusted IP*
/// authentication.  See the [Authorization](../../access_control.md) documentation
/// for more information on adjusting ACLs.
///
/// See also [metrics.json](metrics.json_get.md).
///
/// ## Metric Documentation
///
/// * [Metrics exported by kumod](../../metrics/kumod/index.md)
///
/// ## Example Data
///
/// Here's an example of the shape of the data.  The precise set of
/// counters will vary as we continue to enhance KumoMTA.
///
/// You can see the current list by querying the endpoint with no arguments:
///
/// ```console
/// $ curl http://localhost:8000/metrics
/// ```
///
/// ```txt
/// {% include "reference/http/sample-metrics.txt" %}
/// ```
#[utoipa::path(get, path = "/metrics", responses(
        (status = 200, content_type="text/plain")
))]
async fn report_metrics(Query(params): Query<PrometheusMetricsParams>) -> impl IntoResponse {
    StreamBodyAsOptions::new()
        .content_type(HttpHeaderValue::from_static("text/plain; charset=utf-8"))
        .text(kumo_prometheus::registry::Registry::stream_text(
            params.prefix.clone(),
        ))
}

/// Returns the current set of metrics in a json representation.
/// This is easier to consume than the Prometheus Exposition format, but
/// is more resource intensive to produce and parse when the number of
/// metrics is large, such as for a busy server.
///
/// !!! note
///     Metrics generally represent data at the current point in time,
///     to be consumed by an external system (such as Prometheus) which
///     then in turn can build time series data around those metrics.
///
///     In addition, in order to avoid unbounded RAM usage for systems
///     with many queues, a number of queue- or service-specific metrics
///     will be automatically pruned away when the corresponding queue
///     idles out for a period of time.
///
/// In the default configuration, access to this endpoint requires *Trusted IP*
/// authentication.  See the [Authorization](../../access_control.md) documentation
/// for more information on adjusting ACLs.
///
/// See also [metrics](metrics_get.md).
///
/// ## Metric Documentation
///
/// * [Metrics exported by kumod](../../metrics/kumod/index.md)
///
/// ## Example Data
///
/// Here's an example of the shape of the data.  The precise set of
/// counters will vary as we continue to enhance KumoMTA:
///
/// ```json
/// {% include "reference/http/sample-metrics.json" %}
/// ```
#[utoipa::path(get, path = "/metrics.json", responses(
    (status = 200, content_type="application/json")
))]
async fn report_metrics_json() -> impl IntoResponse {
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
    tags=["logging", "kcli:set-log-filter"],
    path="/api/admin/set_diagnostic_log_filter/v1",
    request_body=SetDiagnosticFilterRequest,
    responses(
        (status = 200, description = "Diagnostic level set successfully")
    ),
)]
async fn set_diagnostic_log_filter_v1(
    // Note: Json<> must be last in the param list
    Json(request): Json<SetDiagnosticFilterRequest>,
) -> Result<(), AppError> {
    set_diagnostic_log_filter(&request.filter)?;
    Ok(())
}
