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
    let msg = Message::new_with_id(request.id.into()).await?;

    let recipient = msg.recipient()?.to_string();
    let sender = msg.sender()?.to_string();
    let meta = msg.get_meta_obj()?;

    let data = if request.want_body {
        msg.load_data_if_needed().await?;
        Some(String::from_utf8_lossy(&msg.get_data()).into())
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
        },
    }))
}
