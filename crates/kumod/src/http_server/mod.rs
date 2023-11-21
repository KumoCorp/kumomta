use axum::routing::{delete, get, post};
use axum::Router;
use inject_v1::*;
use kumo_api_types::*;
use kumo_server_common::http_server::RouterAndDocs;
use spool::SpoolId;
use utoipa::OpenApi;

pub mod admin_bounce_v1;
pub mod admin_inspect_message;
pub mod admin_suspend_ready_q_v1;
pub mod admin_suspend_v1;
pub mod admin_trace_smtp_server_v1;
pub mod inject_v1;

#[derive(OpenApi)]
#[openapi(
    info(title = "kumod",),
    paths(
        inject_v1::inject_v1,
        admin_bounce_v1::bounce_v1,
        admin_bounce_v1::bounce_v1_list,
        admin_bounce_v1::bounce_v1_delete,
        admin_inspect_message::inspect_v1,
        admin_suspend_ready_q_v1::suspend,
        admin_suspend_ready_q_v1::list,
        admin_suspend_ready_q_v1::delete,
        admin_suspend_v1::suspend,
        admin_suspend_v1::list,
        admin_suspend_v1::delete,
    ),
    components(
        schemas(
            FromHeader,
            Recipient,
            Content,
            Header,
            Attachment,
            InjectV1Request,
            InjectV1Response,
            SpoolId,
            BounceV1Request,
            BounceV1Response,
            BounceV1ListEntry,
            BounceV1CancelRequest,
            InspectMessageV1Response,
            MessageInformation,
            SuspendReadyQueueV1Request,
            SuspendV1Response,
            SuspendReadyQueueV1ListEntry,
            SuspendV1CancelRequest,
            SuspendV1ListEntry,
            SuspendV1Request,
        ),
        responses(InjectV1Response, BounceV1Response, InspectMessageV1Response),
    )
)]
struct ApiDoc;

pub fn make_router() -> RouterAndDocs {
    RouterAndDocs {
        router: Router::new()
            .route("/api/inject/v1", post(inject_v1::inject_v1))
            .route("/api/admin/bounce/v1", post(admin_bounce_v1::bounce_v1))
            .route("/api/admin/bounce/v1", get(admin_bounce_v1::bounce_v1_list))
            .route(
                "/api/admin/bounce/v1",
                delete(admin_bounce_v1::bounce_v1_delete),
            )
            .route("/api/admin/suspend/v1", post(admin_suspend_v1::suspend))
            .route("/api/admin/suspend/v1", get(admin_suspend_v1::list))
            .route("/api/admin/suspend/v1", delete(admin_suspend_v1::delete))
            .route(
                "/api/admin/suspend-ready-q/v1",
                post(admin_suspend_ready_q_v1::suspend),
            )
            .route(
                "/api/admin/suspend-ready-q/v1",
                get(admin_suspend_ready_q_v1::list),
            )
            .route(
                "/api/admin/suspend-ready-q/v1",
                delete(admin_suspend_ready_q_v1::delete),
            )
            .route(
                "/api/admin/inspect-message/v1",
                get(admin_inspect_message::inspect_v1),
            )
            .route(
                "/api/admin/trace-smtp-server/v1",
                get(admin_trace_smtp_server_v1::trace),
            ),
        docs: ApiDoc::openapi(),
    }
}
