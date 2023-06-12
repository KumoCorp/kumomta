use crate::runtime::spawn;
use anyhow::Context;
use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use cidr_map::{AnyIpCidr, CidrSet};
use data_loader::KeySource;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::str::FromStr;
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
    pub trusted_hosts: CidrSet,
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
    pub async fn start(self) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/metrics", get(report_metrics))
            .route("/metrics.json", get(report_metrics_json))
            .route("/api/inject/v1", post(inject_v1::inject_v1))
            .route("/api/admin/bounce/v1", post(admin_bounce_v1::bounce_v1))
            .route("/api/admin/bounce/v1", get(admin_bounce_v1::bounce_v1_list))
            .route(
                "/api/admin/bounce/v1",
                delete(admin_bounce_v1::bounce_v1_delete),
            )
            .route(
                "/api/admin/set_diagnostic_log_filter/v1",
                post(set_diagnostic_log_filter_v1),
            )
            // Require that all requests be authenticated as either coming
            // from a trusted IP address, or with an authorization header
            .route_layer(axum::middleware::from_fn_with_state(
                AppState {
                    trusted_hosts: Arc::new(self.trusted_hosts.clone()),
                },
                auth_middleware,
            ));
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

async fn set_diagnostic_log_filter_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(request): Json<kumo_api_types::SetDiagnosticFilterRequest>,
) -> Result<(), AppError> {
    crate::set_diagnostic_log_filter(&request.filter)?;
    Ok(())
}
