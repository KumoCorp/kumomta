use crate::ready_queue::ReadyQueueManager;
use axum::extract::{Json, Query};
use kumo_api_types::{InspectReadyQV1Request, InspectReadyQV1Response};
use kumo_server_common::http_server::AppError;
use reqwest::StatusCode;

/// Retrieve a snapshot of a ready queue's state, configuration, and
/// the dispatcher tasks that are currently handling connections on
/// its behalf.
///
/// {{since('dev')}}
#[utoipa::path(
    get,
    tags=["inspect", "kcli:inspect-ready-q"],
    path="/api/admin/inspect-ready-q/v1",
    params(InspectReadyQV1Request),
    responses(
        (status = 200, description = "Ready queue snapshot", body=InspectReadyQV1Response),
        (status = 404, description = "No such ready queue"),
    ),
)]
pub async fn inspect_v1(
    Query(request): Query<InspectReadyQV1Request>,
) -> Result<Json<InspectReadyQV1Response>, AppError> {
    let Some(queue) = ReadyQueueManager::get_by_name(&request.queue_name) else {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("no such ready queue {}", request.queue_name),
        ));
    };
    Ok(Json(
        queue.build_inspect_response(request.include_scheduled_queues),
    ))
}
