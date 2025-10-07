use crate::queue::QueueManager;
use axum::extract::{Json, Query};
use kumo_api_types::{
    InspectMessageV1Response, InspectQueueV1Request, InspectQueueV1Response, MessageInformation,
};
use kumo_chrono_helper::Utc;
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;
use reqwest::StatusCode;

/// Retrieve information about messages in a scheduled queue.
#[utoipa::path(
    get,
    tag="inspect",
    path="/api/admin/inspect-sched-q/v1",
    params(InspectQueueV1Request),
    responses(
        (status = 200, description = "Obtained queue information", body=InspectQueueV1Response),
    ),
)]
pub async fn inspect_v1(
    _: TrustedIpRequired,
    Query(request): Query<InspectQueueV1Request>,
) -> Result<Json<InspectQueueV1Response>, AppError> {
    let Some(queue) = QueueManager::get_opt(&request.queue_name) else {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("no such queue {}", request.queue_name),
        ));
    };
    let mut messages = vec![];

    for msg in queue.iter(request.limit) {
        if msg.load_meta_if_needed().await.is_ok() {
            let recipient = msg.recipient_list_string()?;
            let sender = msg.sender()?.to_string();
            let meta = msg.get_meta_obj()?;
            let scheduling = msg
                .get_scheduling()
                .and_then(|s| serde_json::to_value(s).ok());

            let data = if request.want_body {
                msg.load_data_if_needed().await?;
                Some(String::from_utf8_lossy(&msg.get_data()).into())
            } else {
                None
            };

            let due = msg.get_due();
            let num_attempts = msg.get_num_attempts();

            messages.push(InspectMessageV1Response {
                id: *msg.id(),
                message: MessageInformation {
                    sender,
                    recipient,
                    meta,
                    data,
                    due,
                    num_attempts: Some(num_attempts),
                    scheduling,
                },
            });

            let _ = msg.shrink();
        }
    }

    let metrics = queue.metrics();
    let now = Utc::now();

    Ok(Json(InspectQueueV1Response {
        queue_name: request.queue_name,
        messages,
        num_scheduled: queue.queue_len(),
        queue_config: serde_json::to_value(&**queue.get_config().borrow())?,
        delayed_metric: metrics.scheduled.delay_gauge.get(),
        last_changed: now - queue.get_last_change().elapsed(),
        now,
    }))
}
