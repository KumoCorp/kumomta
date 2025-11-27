use axum::extract::{Json, Query};
use kumo_api_types::{InspectMessageV1Request, InspectMessageV1Response, MessageInformation};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;
use message::Message;

/// Retrieve information about a message given its spool id.
#[utoipa::path(
    get,
    tag="inspect",
    path="/api/admin/inspect-message/v1",
    params(InspectMessageV1Request),
    responses(
        (status = 200, description = "Obtained message information", body=InspectMessageV1Response),
    ),
)]
pub async fn inspect_v1(
    _: TrustedIpRequired,
    Query(request): Query<InspectMessageV1Request>,
) -> Result<Json<InspectMessageV1Response>, AppError> {
    let msg = Message::new_with_id(request.id).await?;

    let recipient = msg.recipient_list_string().await?;
    let sender = msg.sender().await?.to_string();
    let meta = msg.get_meta_obj().await?;
    let scheduling = msg
        .get_scheduling()
        .await?
        .and_then(|s| serde_json::to_value(s).ok());

    let data = if request.want_body {
        Some(String::from_utf8_lossy(&msg.data().await?).into())
    } else {
        None
    };

    Ok(Json(InspectMessageV1Response {
        id: request.id,
        message: MessageInformation {
            sender,
            recipient,
            meta,
            data,
            due: None,
            num_attempts: None,
            scheduling,
        },
    }))
}
