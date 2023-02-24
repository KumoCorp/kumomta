use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Router, Server};
use serde::Deserialize;
use std::net::{SocketAddr, TcpListener};

#[derive(Deserialize, Clone, Debug)]
pub struct HttpListenerParams {
    #[serde(default = "HttpListenerParams::default_listen")]
    pub listen: String,
}

impl HttpListenerParams {
    fn default_listen() -> String {
        "127.0.0.1:8000".to_string()
    }

    pub async fn start(self) -> anyhow::Result<()> {
        let app = Router::new().route("/metrics", get(report_metrics));
        let addr: SocketAddr = self.listen.parse()?;
        let socket = TcpListener::bind(&self.listen)?;
        tracing::debug!("http listener on {addr:?}");
        let server = Server::from_tcp(socket)?;
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
        tokio::spawn(async move { server.serve(app.into_make_service()).await });
        Ok(())
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

async fn report_metrics() -> Result<String, AppError> {
    let report = prometheus::TextEncoder::new()
        .encode_to_string(&prometheus::default_registry().gather())?;
    Ok(report)
}
