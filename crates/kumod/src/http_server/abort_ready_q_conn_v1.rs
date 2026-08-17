use crate::ready_queue::ReadyQueueManager;
use axum::extract::Json;
use axum::response::{IntoResponse, Response};
use kumo_api_types::AbortReadyQConnV1Request;
use reqwest::StatusCode;

/// Abort the dispatcher task identified by `session_id` within the
/// named ready queue.
///
/// {{since('dev')}}
///
/// Returns 404 if either the queue or the session is unknown.
#[utoipa::path(
    post,
    tags=["inspect", "kcli:abort-ready-q-conn"],
    path="/api/admin/abort-ready-q-conn/v1",
    request_body=AbortReadyQConnV1Request,
    responses(
        (status = 200, description = "Aborted"),
        (status = 404, description = "No matching queue or session"),
    ),
)]
pub async fn abort_v1(Json(request): Json<AbortReadyQConnV1Request>) -> Response {
    let aborted = match ReadyQueueManager::get_by_name(&request.queue_name) {
        Some(queue) => queue.abort_dispatcher_by_session(request.session_id),
        None => false,
    };
    if aborted {
        (StatusCode::OK, format!("aborted {}", request.session_id)).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            format!(
                "no matching dispatcher for queue={} session={}",
                request.queue_name, request.session_id
            ),
        )
            .into_response()
    }
}
