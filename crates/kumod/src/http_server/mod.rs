use axum::routing::{delete, get, post};
use axum::Router;

pub mod admin_bounce_v1;
pub mod admin_inspect_message;
pub mod admin_suspend_ready_q_v1;
pub mod admin_suspend_v1;
pub mod inject_v1;

pub fn make_router() -> Router {
    Router::new()
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
}
