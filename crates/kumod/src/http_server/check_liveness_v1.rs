use crate::spool::SpoolManager;
use axum::response::Response;
use kumo_server_lifecycle::Activity;

/// Useful for load balancers to determine when service is available
/// and ready to receive messages
#[utoipa::path(
    get,
    tag="liveness",
    path="/api/check-liveness/v1",
    responses(
        (status = 200, description = "ready to accept messages"),
        (status = 503, description = "service is not currently available"),
    ),
)]
pub async fn check_liveness_v1() -> Response {
    let (status, content) = match Activity::get_opt("check liveness".to_string()) {
        None => (503, "shutting down"),
        Some(_activity) => {
            if kumo_server_memory::get_headroom() == 0 {
                (503, "load shedding")
            } else if !SpoolManager::get().spool_started() {
                (503, "waiting for spool startup")
            } else if kumo_server_common::disk_space::is_over_limit() {
                (503, "storage is too full")
            } else {
                (200, "OK")
            }
        }
    };
    Response::builder()
        .status(status)
        .body(content.into())
        .unwrap()
}
