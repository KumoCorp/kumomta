use kumo_server_common::http_server::RouterAndDocs;
use kumo_server_common::router_with_docs;
use utoipa::OpenApi;

pub mod admin_bounce_v1;
pub mod admin_inspect_message;
pub mod admin_inspect_scheduled_queue;
pub mod admin_ready_queue_states;
pub mod admin_rebind_v1;
pub mod admin_suspend_ready_q_v1;
pub mod admin_suspend_v1;
pub mod admin_trace_smtp_client_v1;
pub mod admin_trace_smtp_server_v1;
pub mod check_liveness_v1;
pub mod inject_v1;
pub mod queue_name_multi_index;

pub fn make_router() -> RouterAndDocs {
    router_with_docs!(
        title = "kumod",
        handlers = [
            admin_bounce_v1::bounce_v1,
            admin_bounce_v1::bounce_v1_delete,
            admin_bounce_v1::bounce_v1_list,
            admin_inspect_message::inspect_v1,
            admin_inspect_scheduled_queue::inspect_v1,
            admin_ready_queue_states::readyq_states,
            admin_rebind_v1::rebind_v1,
            admin_suspend_ready_q_v1::delete,
            admin_suspend_ready_q_v1::list,
            admin_suspend_ready_q_v1::suspend,
            admin_suspend_v1::delete,
            admin_suspend_v1::list,
            admin_suspend_v1::suspend,
            admin_trace_smtp_client_v1::trace,
            admin_trace_smtp_server_v1::trace,
            check_liveness_v1::check_liveness_v1,
            crate::xfer::cancel::xfer_cancel_v1,
            crate::xfer::inject_xfer_v1,
            crate::xfer::request::xfer_v1,
            inject_v1::inject_v1,
        ]
    )
}
