//! Batched HTTP injection, preserving the semantics of /api/inject/v1.

use super::inject_v1::{inject_v1, InjectV1Request};
use axum::extract::{Json, State};
use axum_client_ip::ClientIp;
use kumo_api_types::InjectV1Response;
use kumo_server_common::authn_authz::AuthInfo;
use kumo_server_common::http_server::{AppError, AppState};
use reqwest::StatusCode;
use rfc5321::parser::EnvelopeAddress;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// The HTTP listener separately applies its configured request-body byte limit.
const MAX_BATCH_REQUESTS: usize = 100;
const MAX_BATCH_RECIPIENTS: usize = 10_000;

/// Each request has the same schema as POST /api/inject/v1.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InjectBatchV1Request {
    pub requests: Vec<InjectV1Request>,
}

/// The outcome of a single entry, identified by its input array index.
#[derive(Serialize, ToSchema)]
pub struct InjectBatchV1EntryResult {
    pub index: usize,
    /// Status of the underlying single-request injection call.
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<InjectV1Response>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct InjectBatchV1Response {
    pub results: Vec<InjectBatchV1EntryResult>,
}

fn validate_counts(request_count: usize, recipient_count: usize) -> Result<(), AppError> {
    if request_count == 0 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "requests must not be empty",
        ));
    }
    if request_count > MAX_BATCH_REQUESTS {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("a batch may contain at most {MAX_BATCH_REQUESTS} injection requests"),
        ));
    }
    if recipient_count > MAX_BATCH_RECIPIENTS {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("a batch may contain at most {MAX_BATCH_RECIPIENTS} recipients"),
        ));
    }
    Ok(())
}

/// Validate all addresses before the first message is enqueued.
///
/// This prevents a malformed address in a later entry from producing
/// avoidable partial admission. Content rendering and queue failures retain
/// the per-entry and per-recipient semantics of /api/inject/v1.
fn validate_addresses(batch: &InjectBatchV1Request) -> Result<(), AppError> {
    for (index, request) in batch.requests.iter().enumerate() {
        EnvelopeAddress::parse(&request.envelope_sender).map_err(|err| {
            AppError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("requests[{index}].envelope_sender: {err:#}"),
            )
        })?;
        for (recipient_index, recipient) in request.recipients.iter().enumerate() {
            EnvelopeAddress::parse(&recipient.email).map_err(|err| {
                AppError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("requests[{index}].recipients[{recipient_index}].email: {err:#}"),
                )
            })?;
        }
    }
    Ok(())
}

/// Inject independently rendered messages in a single HTTP POST.
///
/// Body: {"requests": [<InjectV1Request>, ...]}.
///
/// The entire batch is rejected if any sender or recipient address is
/// invalid. After address validation, the operation is non-atomic. An outer
/// 200 response can contain individual failures, reported in input order.
/// When Kumo rejects an entry with
/// HTTP 503, all remaining entries are explicitly marked as not attempted.
/// Retry only failed entries/recipients; avoid replaying successful ones.
/// A transport failure with no response is ambiguous.
/// In deferred_generation mode, success_count=0 retains its existing meaning:
/// the request was queued for later generation, not expanded immediately.
///
/// Each entry is processed through the existing HTTP injection handler, which
/// retains its activity checks, recipient throttling, concurrency controls,
/// queuing and response behavior.
#[utoipa::path(
    post,
    tag = "inject",
    path = "/api/inject/batch/v1",
    request_body = InjectBatchV1Request,
    responses(
        (
            status = 200,
            description = "Ordered injection results (possibly with per-entry errors)",
            body = InjectBatchV1Response
        ),
        (status = 400, description = "Invalid or empty batch"),
        (status = 413, description = "Request count, recipient count or body size exceeded"),
        (status = 422, description = "An entry contains a malformed sender or recipient address"),
    ),
)]
pub async fn inject_batch_v1(
    auth: AuthInfo,
    ClientIp(peer_address): ClientIp,
    State(app_state): State<AppState>,
    // Json must be the last extractor.
    Json(batch): Json<InjectBatchV1Request>,
) -> Result<Json<InjectBatchV1Response>, AppError> {
    let recipient_count = batch.requests.iter().fold(0usize, |sum, request| {
        sum.saturating_add(request.recipients.len())
    });
    validate_counts(batch.requests.len(), recipient_count)?;
    validate_addresses(&batch)?;

    // Intentionally sequential to avoid multiplying concurrency/memory usage
    // beyond the controls of the existing single-request injection endpoint.
    let request_count = batch.requests.len();
    let mut results = Vec::with_capacity(request_count);
    for (index, request) in batch.requests.into_iter().enumerate() {
        match inject_v1(
            auth.clone(),
            ClientIp(peer_address),
            State(app_state.clone()),
            Json(request),
        )
        .await
        {
            Ok(Json(response)) => results.push(InjectBatchV1EntryResult {
                index,
                status: StatusCode::OK.as_u16(),
                response: Some(response),
                error: None,
            }),
            Err(err) => {
                let overloaded = err.code == StatusCode::SERVICE_UNAVAILABLE;
                results.push(InjectBatchV1EntryResult {
                    index,
                    status: err.code.as_u16(),
                    response: None,
                    error: Some(format!("{:#}", err.err)),
                });

                // Avoid repeatedly submitting work after the underlying
                // handler has rejected admission (shutdown, disk/memory
                // pressure, or injection concurrency saturation).
                // Explicitly report every unattempted entry so the caller
                // can retry those positions without replaying successes.
                if overloaded {
                    for skipped_index in (index + 1)..request_count {
                        results.push(InjectBatchV1EntryResult {
                            index: skipped_index,
                            status: StatusCode::SERVICE_UNAVAILABLE.as_u16(),
                            response: None,
                            error: Some(
                                "not attempted: previous entry returned HTTP 503".to_string(),
                            ),
                        });
                    }
                    break;
                }
            }
        }
    }

    Ok(Json(InjectBatchV1Response { results }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_independent_requests() {
        let batch: InjectBatchV1Request = serde_json::from_value(json!({
            "requests": [
                {
                    "envelope_sender": "sender@example.com",
                    "content": "Subject: A\n\nA",
                    "recipients": [{"email": "a@example.com"}]
                },
                {
                    "envelope_sender": "sender@example.com",
                    "content": "Subject: B\n\nB",
                    "recipients": [{"email": "b@example.com"}]
                }
            ]
        }))
        .unwrap();
        assert_eq!(batch.requests.len(), 2);
        validate_counts(2, 2).unwrap();
    }

    #[test]
    fn rejects_unexpected_top_level_fields() {
        assert!(serde_json::from_value::<InjectBatchV1Request>(json!([])).is_err());
        assert!(serde_json::from_value::<InjectBatchV1Request>(json!({
            "requests": [],
            "unexpected": true
        }))
        .is_err());
    }

    #[test]
    fn enforces_count_limits() {
        assert_eq!(
            validate_counts(0, 0).unwrap_err().code,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            validate_counts(MAX_BATCH_REQUESTS + 1, 1).unwrap_err().code,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            validate_counts(1, MAX_BATCH_RECIPIENTS + 1).unwrap_err().code,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        validate_counts(MAX_BATCH_REQUESTS, MAX_BATCH_RECIPIENTS).unwrap();
    }

    #[test]
    fn rejects_invalid_addresses_before_processing() {
        let batch: InjectBatchV1Request = serde_json::from_value(json!({
            "requests": [
                {
                    "envelope_sender": "sender@example.com",
                    "content": "Subject: One\\n\\nOne",
                    "recipients": [{"email": "one@example.com"}]
                },
                {
                    "envelope_sender": "sender@example.com",
                    "content": "Subject: Two\\n\\nTwo",
                    "recipients": [{"email": "not-an-email"}]
                }
            ]
        }))
        .unwrap();

        let err = validate_addresses(&batch).unwrap_err();
        assert_eq!(err.code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(format!("{:#}", err.err).contains("requests[1].recipients[0].email"));
    }

    #[test]
    fn serializes_positional_errors() {
        let response = InjectBatchV1Response {
            results: vec![InjectBatchV1EntryResult {
                index: 1,
                status: 422,
                response: None,
                error: Some("bad template".into()),
            }],
        };
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["results"][0]["index"], 1);
        assert_eq!(json["results"][0]["status"], 422);
        assert!(json["results"][0].get("response").is_none());
    }
}
