use axum::routing::post;
use axum::{Json, Router};
use kumo_log_types::*;
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;

pub fn make_router() -> Router {
    Router::new().route("/publish_log_v1", post(publish_log_v1))
}

async fn publish_log_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(record): Json<JsonLogRecord>,
) -> Result<(), AppError> {
    tracing::info!("got record: {record:?}");
    Ok(())
}
