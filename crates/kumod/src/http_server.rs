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
