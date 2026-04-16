use crate::queue::QueueManager;
use axum::extract::Json;
use kumo_api_types::xfer::{XferCancelV1Request, XferCancelV1Response, XferProtocol};
use kumo_server_common::http_server::AppError;
use kumo_server_runtime::rt_spawn;
use reqwest::StatusCode;

/// Allows the system operator to stop a message transfer that was
/// previously initiated via the `/api/admin/xfer/v1` API endpoint.
/// The cancellation works by walking the xfer scheduled queue,
/// reversing the metadata changes made as part of setting up the
/// xfer (to restore the original scheduling information) and then
/// re-inserting the message into an appropriate scheduled queue.
///
/// The cancellation is "instantaneous" in the sense that it applies
/// just once to the specified xfer scheduled queue.  Any other messages
/// that are in-flight or imminently about to be reinserted into
/// that scheduled queue will not be considered, so you may need
/// to trigger the cancellation a few times over short time span
/// to ensure that all messages are taken out of the xfer queue.
///
/// Cancellation requests always complete asynchronously because
/// they may operate on very large quantities of messages, and it
/// is infeasible to wait for completion in the context of
/// a single HTTP request.
#[utoipa::path(
    post,
    tag="xfer",
    path="/api/admin/xfer/cancel/v1",
    responses(
        (status = 200, description = "Xfer added successfully", body=XferCancelV1Response)
    ),
)]
pub async fn xfer_cancel_v1(
    // Note: Json<> must be last in the param list
    Json(request): Json<XferCancelV1Request>,
) -> Result<Json<XferCancelV1Response>, AppError> {
    if !XferProtocol::is_xfer_queue_name(&request.queue_name) {
        // Can't cancel an xfer if the queue is not an xfer queue!
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("{} is not an xfer queue", request.queue_name),
        ));
    }

    let Ok(queue) = QueueManager::resolve(&request.queue_name).await else {
        // If there is no matching queue, there's nothing to cancel
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("{} is not a current scheduled queue", request.queue_name),
        ));
    };

    rt_spawn("process_xfer_cancel_v1".to_string(), async move {
        queue.cancel_xfer_all(request.reason).await;
    })?;

    Ok(Json(XferCancelV1Response {}))
}
